//! ISO-BMFF (MP4/MOV) metadata carriers.
//!
//! This module answers "what does this container say about when and where the
//! recording happened, and where are those bytes". It walks the box tree of the
//! `moov` box and resolves the carriers QuickTime, Apple devices and ffmpeg
//! write:
//!
//! - `moov/mvhd` creation time (seconds from the 1904-01-01 QuickTime epoch),
//! - the mdta pair `moov/udta/meta/keys` (key names) + `moov/udta/meta/ilst`
//!   (values indexed into those names),
//! - legacy `©day` / `©xyz` item types in `moov/udta/meta/ilst`,
//! - a direct `moov/udta/©day` / `moov/udta/©xyz` child,
//! - an ISO/3GPP `moov/udta/loci` location (a 16.16 fixed-point pair).
//!
//! The parser is hand-rolled over `std` on purpose: a general-purpose demuxer
//! returns values, not the byte spans an in-place rewrite needs, and this
//! project adds no dependency for it. Every failure path is an `Err` — a
//! malformed container is never allowed to panic or to size an allocation.
//!
//! Only `moov` is ever read from the file: `mdat` is located by its header and
//! skipped.

use std::collections::HashMap;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, NaiveTime, Timelike, Utc};

/// Extensions this project will rewrite in place.
pub const WRITABLE_EXTENSIONS: [&str; 3] = ["mp4", "mov", "m4v"];

/// Boxes whose payload is a plain sequence of child boxes. Everything else is
/// treated as a leaf: parsing sample tables (`stsd` and below) would cost time
/// and buy nothing, the metadata carriers never live there.
const CONTAINER_BOXES: [[u8; 4]; 11] = [
    *b"moov", *b"trak", *b"mdia", *b"minf", *b"stbl", *b"udta", *b"edts", *b"mvex", *b"moof",
    *b"traf", *b"ilst",
];

/// `meta` is the one container whose payload needs a heuristic: ISO-BMFF
/// declares it a FullBox (4-byte version/flags ahead of the children),
/// QuickTime writes the children directly. See [`meta_children_offset`].
const META_BOX: [u8; 4] = *b"meta";

/// Legacy QuickTime item type carrying a date.
const DAY_BOX: [u8; 4] = [0xA9, b'd', b'a', b'y'];

/// [`DAY_BOX`] as text: how a failure names the carrier it could not fill.
const DAY_NAME: &str = "\u{a9}day";

/// Legacy QuickTime item type carrying an ISO 6709 location.
const XYZ_BOX: [u8; 4] = [0xA9, b'x', b'y', b'z'];

/// [`XYZ_BOX`] as text: how a failure names the legacy location carrier.
const XYZ_NAME: &str = "\u{a9}xyz";

/// The ISO/3GPP `udta/loci` location carrier, as text: how a failure names it.
const LOCI_NAME: &str = "loci";

/// How a refusal names the date when the container holds no date carrier at all:
/// the chain is the `mvhd`/`tkhd`/`mdhd` fields together with every text date
/// carrier, so there is no single box to point at.
const DATE_CARRIER: &str = "date";

/// Scale of the 16.16 fixed-point pair a `loci` box stores: each horizontal
/// component is an `i32` counting 1/65536 of a degree.
const LOCI_FIXED_POINT_SCALE: f64 = 65_536.0;

/// The one `loci` role this reader treats as a position: the place itself. The
/// roles above it (1..=8: room, building, floor, postcode, intersection,
/// island, street, address) name a region the box does not give the extent of.
const LOCI_ROLE_EXACT: u8 = 0;

/// mdta key names that carry the creation date, in resolution order.
const MDTA_CREATION_DATE_KEYS: [&str; 2] = ["com.apple.quicktime.creationdate", "creation_time"];

/// mdta key names that carry the ISO 6709 location, in resolution order.
const MDTA_LOCATION_KEYS: [&str; 2] = ["com.apple.quicktime.location.ISO6709", "location"];

/// Seconds from the QuickTime epoch (1904-01-01T00:00:00Z) to the Unix epoch.
/// The QuickTime epoch predates it, so the offset is subtracted from a stored
/// value to reach a Unix timestamp.
const QUICKTIME_EPOCH_UNIX_SECONDS: i64 = 2_082_844_800;

/// Largest `moov` this reader will pull into memory. A `moov` is metadata, at
/// most a few MiB even for a long recording; a larger declared size is corrupt
/// or hostile, and the reader must not size an allocation from it.
const MAX_MOOV_BYTES: u64 = 64 * 1024 * 1024;

/// Deepest container nesting the walker follows. Real files stay under 10
/// levels (`moov/udta/meta/ilst/item/data`); the cap costs one comparison per
/// box and keeps a crafted file from recursing until the stack dies.
const MAX_BOX_DEPTH: usize = 32;

/// Largest number of boxes one walk will turn into a span. A box costs at
/// least 8 bytes in the buffer and a `BoxSpan` (with its own `children` vector)
/// costs a multiple of that, so a `moov` under [`MAX_MOOV_BYTES`] that holds
/// nothing but minimal headers would otherwise turn into millions of spans and
/// dwarf the buffer it was read from. Real containers hold tens: a two-track
/// file reaches forty, a hundred-track multicam stays in the low thousands, and
/// this is an order of magnitude above that.
const MAX_BOXES: usize = 1 << 16;

/// Smallest `keys` entry the format allows: a `size` that covers itself and the
/// 4-byte namespace, with no key bytes behind it. Bounds the entry walk by what
/// the box can physically hold.
const MIN_MDTA_ENTRY_BYTES: usize = 8;

/// Largest number of key names one `keys` box may contribute. A name is only
/// ever reached through an `ilst` item carrying its 1-based position, and the
/// whole item list is bounded by [`MAX_BOXES`] — so a longer name list cannot be
/// indexed in full by anything this reader resolves, while a `keys` box packed
/// with minimum-size entries would otherwise name one `String` per eight bytes
/// of `moov`, on every read of every video on every scan.
const MAX_MDTA_KEYS: usize = MAX_BOXES;

/// The metadata a container exposes, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mp4Metadata {
    /// `mvhd` creation time, as stored. `None` when the container says no
    /// instant: no `mvhd` at all, one that stores 0 ("never set" — ffprobe
    /// reports no `creation_time` for those), or one this reader cannot
    /// decode, which it answers with a warning and the text carriers alone.
    /// The date itself is never wrong — a save writes every carrier, and the
    /// scanner falls back the way it does for a file with no movie header.
    pub creation_time: Option<DateTime<Utc>>,
    /// Textual creation date, verbatim from whichever carrier held it.
    pub creation_date_text: Option<String>,
    /// ISO 6709 location, verbatim from whichever carrier held it.
    pub location_iso6709: Option<String>,
}

/// Why a container's metadata could not be read.
#[derive(Debug)]
pub enum Mp4MetadataError {
    /// Not an ISO-BMFF file, or an ISO-BMFF file that carries no metadata the
    /// reader understands. The payload names what was found instead.
    UnsupportedContainer(String),
    /// A fragmented movie (`mvex` present): its metadata lives in fragments.
    Fragmented,
    /// No carrier in the file holds a location.
    NoLocationCarrier,
    /// The named box cannot hold the bytes it would have to hold.
    NoRoom(&'static str),
    /// The named value cannot be encoded in this container.
    Unrepresentable(&'static str),
    /// A date field decodes outside the representable range.
    InvalidDate,
    /// Coordinates are outside the valid ranges.
    InvalidCoordinates,
    /// The file does not exist.
    MissingFile,
    /// The file may not be written.
    ReadOnly(String),
    /// Reading the file failed.
    Io(std::io::Error),
}

impl fmt::Display for Mp4MetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedContainer(found) => write!(f, "Unsupported container: {found}"),
            Self::Fragmented => write!(f, "Fragmented movies carry their metadata in fragments"),
            Self::NoLocationCarrier => write!(f, "No location carrier in the container"),
            Self::NoRoom(what) => write!(f, "Not enough room for {what}"),
            Self::Unrepresentable(what) => write!(f, "Value cannot be represented: {what}"),
            Self::InvalidDate => write!(f, "Invalid creation date"),
            Self::InvalidCoordinates => write!(f, "Invalid coordinates"),
            Self::MissingFile => write!(f, "File not found"),
            Self::ReadOnly(what) => write!(f, "File is read-only: {what}"),
            Self::Io(err) => write!(f, "I/O error: {err}"),
        }
    }
}

impl std::error::Error for Mp4MetadataError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

/// Whether `path` names a container this project rewrites in place. Matching is
/// case-insensitive: `.MOV` off a camera is the same container as `.mov`.
pub fn has_writable_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .is_some_and(|extension| WRITABLE_EXTENSIONS.contains(&extension.as_str()))
}

/// One box in the tree: where it is, how big it is, what it is, and — for
/// containers — what it holds.
#[derive(Debug, Clone)]
pub(crate) struct BoxSpan {
    /// Offset of the box header (its size field) in the buffer it was parsed
    /// from.
    pub offset: usize,
    /// Total box size in bytes, header included.
    pub size: usize,
    /// The four-character box type.
    pub kind: [u8; 4],
    /// Child boxes, in file order; empty for a leaf box.
    pub children: Vec<BoxSpan>,
}

/// Parses the boxes in `buf[start..end]`.
///
/// `path` is the chain of container types being descended through, and its last
/// element names the box whose payload the region is — which is what tells a
/// bare QuickTime `meta` body apart from an ISO-BMFF FullBox body. A caller
/// parsing a whole file (or a whole `moov` region) passes `&[]`.
///
/// Boxes whose declared size runs past `end`, past the buffer, or below the
/// smallest legal box are an error rather than a silent truncation.
pub(crate) fn parse_box_tree(
    buf: &[u8],
    start: usize,
    end: usize,
    path: &[[u8; 4]],
) -> Result<Vec<BoxSpan>, Mp4MetadataError> {
    let mut enclosing = path.to_vec();
    let mut budget = MAX_BOXES;
    parse_region(buf, start, end.min(buf.len()), &mut enclosing, &mut budget)
}

/// [`parse_box_tree`] over a reusable `enclosing` chain, so descending into a
/// container costs a push/pop instead of a fresh path vector per box.
fn parse_region(
    buf: &[u8],
    start: usize,
    end: usize,
    enclosing: &mut Vec<[u8; 4]>,
    budget: &mut usize,
) -> Result<Vec<BoxSpan>, Mp4MetadataError> {
    if enclosing.len() > MAX_BOX_DEPTH {
        return Err(Mp4MetadataError::UnsupportedContainer(
            "box nesting depth".to_string(),
        ));
    }
    let start = start.min(end);
    let mut offset = start;
    if enclosing.last() == Some(&META_BOX) {
        offset = (offset + meta_children_offset(buf, start, end)).min(end);
    }
    let mut spans = Vec::new();
    // `end - offset` rather than `offset + 8 <= end`: the difference cannot
    // overflow, and `offset <= end` is the invariant every later subtraction
    // relies on.
    while end - offset >= 8 {
        // One box of the whole walk's budget, counted across every level, so
        // nesting cannot buy a file more spans than a flat one.
        let Some(remaining) = budget.checked_sub(1) else {
            return Err(Mp4MetadataError::UnsupportedContainer(
                "box count".to_string(),
            ));
        };
        *budget = remaining;
        let available = end - offset;
        let declared = size_field(buf, offset).ok_or_else(unsupported_size)?;
        // Every declared size is validated against `available` before it reaches
        // an addition. The 64-bit `largesize` form can declare a size near
        // `usize::MAX`, which an unchecked `offset + size` would wrap past the
        // bounds check — turning a malformed box into an endless walk.
        let (size, header) = match declared {
            // "Extends to the end of the enclosing box" — the file's own size
            // for a top-level box.
            0 => (available, 8),
            1 if available >= 16 => {
                let large = buf
                    .get(offset + 8..offset + 16)
                    .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
                    .ok_or_else(unsupported_size)?;
                (
                    usize::try_from(u64::from_be_bytes(large)).map_err(|_| unsupported_size())?,
                    16,
                )
            }
            // Also covers `size == 1` with a truncated `largesize`: 1 is below
            // the 8-byte header floor, so the guard below rejects it.
            size => (size as usize, 8),
        };
        if size < header || size > available {
            return Err(unsupported_size());
        }
        let mut kind = [0u8; 4];
        if let Some(bytes) = buf.get(offset + 4..offset + 8) {
            kind.copy_from_slice(bytes);
        }
        let children = if is_container(&kind, enclosing) {
            enclosing.push(kind);
            let parsed = parse_region(buf, offset + header, offset + size, enclosing, budget);
            enclosing.pop();
            parsed?
        } else {
            Vec::new()
        };
        spans.push(BoxSpan {
            offset,
            size,
            kind,
            children,
        });
        offset += size;
    }
    Ok(spans)
}

/// Whether `kind` holds child boxes. Every child of an `ilst` does: those are
/// item boxes whose type is either an mdta key index or a `©`-prefixed legacy
/// type, both of which wrap a `data` box.
fn is_container(kind: &[u8; 4], enclosing: &[[u8; 4]]) -> bool {
    *kind == META_BOX || CONTAINER_BOXES.contains(kind) || enclosing.last() == Some(b"ilst")
}

/// Where a `meta` box's children start, relative to its payload.
///
/// ISO-BMFF declares `meta` a FullBox, so its payload opens with a version/flags
/// word and the first child header follows 4 bytes in. QuickTime omits that word
/// and starts the payload with the child header itself. The payload is told
/// apart by looking at it: the version/flags word of a real `meta` box is 0,
/// which can never be a box's size field (no box is smaller than 8 bytes), so a
/// plausible child header at the payload start means the QuickTime form.
fn meta_children_offset(buf: &[u8], body: usize, end: usize) -> usize {
    if looks_like_box_header(buf, body, end) {
        0
    } else {
        4
    }
}

/// Whether a child box header starts at `offset`, within the region that ends
/// at `end`.
fn looks_like_box_header(buf: &[u8], offset: usize, end: usize) -> bool {
    if offset + 8 > end {
        return false;
    }
    if size_field(buf, offset).is_none_or(|size| size < 8) {
        return false;
    }
    buf.get(offset + 4..offset + 8)
        .is_some_and(|kind| kind.iter().all(is_type_byte))
}

/// Box types are printable ASCII, except for the `©`-prefixed legacy ones.
fn is_type_byte(byte: &u8) -> bool {
    byte.is_ascii_graphic() || *byte == 0xA9
}

/// The 4-byte big-endian size field of the box at `offset`, when present.
fn size_field(buf: &[u8], offset: usize) -> Option<u32> {
    let bytes = buf.get(offset..offset + 4)?;
    <[u8; 4]>::try_from(bytes).ok().map(u32::from_be_bytes)
}

/// Header length of the box at `offset`: 8 bytes, or 16 with the 64-bit
/// `largesize` form. Only called on offsets [`parse_box_tree`] validated.
fn header_len(buf: &[u8], offset: usize) -> usize {
    match size_field(buf, offset) {
        Some(1) => 16,
        _ => 8,
    }
}

fn unsupported_size() -> Mp4MetadataError {
    Mp4MetadataError::UnsupportedContainer("box size".to_string())
}

/// Reads the metadata carriers of one MP4/MOV file.
///
/// The file is walked by top-level box headers only, so only `moov` is ever
/// read into memory (`mdat` is skipped, however large).
pub fn read_metadata(path: &Path) -> Result<Mp4Metadata, Mp4MetadataError> {
    let mut file = File::open(path).map_err(|err| match err.kind() {
        ErrorKind::NotFound => Mp4MetadataError::MissingFile,
        _ => Mp4MetadataError::Io(err),
    })?;
    let file_len = file.metadata().map_err(Mp4MetadataError::Io)?.len();
    let (moov_offset, moov_size) = locate_moov(&mut file, file_len)?;
    let buf = read_moov(&mut file, moov_offset, moov_size)?;
    let tree = parse_box_tree(&buf, 0, buf.len(), &[])?;
    let moov = tree
        .first()
        .filter(|span| span.kind == *b"moov")
        .ok_or_else(|| Mp4MetadataError::UnsupportedContainer("moov".to_string()))?;
    if contains_kind(moov, b"mvex") {
        return Err(Mp4MetadataError::Fragmented);
    }
    let (creation_date_text, location_iso6709) = text_carriers(&buf, moov);
    Ok(Mp4Metadata {
        // An `mvhd` instant this reader cannot decode costs it the date alone,
        // not the text carriers the same read has already resolved: a file
        // with a malformed movie header still answers with the position its
        // `©xyz`/`loci` boxes hold, and the scanner falls back to those
        // exactly as it does for a file carrying no `mvhd` at all. The instant
        // is then absent, not wrong.
        creation_time: mvhd_creation_time(&buf, moov).unwrap_or_else(|err| {
            log::warn!(
                "could not read the movie header creation time of {} ({err}); reporting the \
                 text carriers without it",
                path.display()
            );
            None
        }),
        creation_date_text,
        location_iso6709,
    })
}

/// Offset and size of the top-level `moov` box.
///
/// Only box headers are read — 8 bytes each, 16 when the size field is 1 — so a
/// file whose `mdat` is gigabytes costs a handful of seeks, not a full read.
/// `size == 0` means the box runs to EOF.
///
/// A file that names no brand is accepted when it leads with a box of the
/// family ([`leads_iso_bmff`]): QuickTime is a defined brand of ISO-BMFF, and
/// a `.mov` from a camera carries no `ftyp` at all.
fn locate_moov(file: &mut File, file_len: u64) -> Result<(u64, u64), Mp4MetadataError> {
    let mut offset = 0u64;
    let mut found_container = false;
    let mut budget = MAX_BOXES;
    // Scan on the bytes remaining (`file_len - offset`) rather than on
    // `offset + 8 <= file_len`: an accepted box advances `offset` by at most
    // what is left, so the difference cannot wrap — and the guard is written so
    // that it cannot underflow either.
    while offset <= file_len && file_len - offset >= 8 {
        // Counted across the whole scan, the way `parse_region` spends its own:
        // a file built entirely of minimum-size placeholder boxes advances
        // eight bytes per header read, and `is_placeholder` exempts exactly
        // those from the sniff — so without a budget the walk costs a seek
        // and a read per eight bytes of the file before it can refuse, on
        // every scan of every video.
        let Some(remaining) = budget.checked_sub(1) else {
            return Err(Mp4MetadataError::UnsupportedContainer(
                "box count".to_string(),
            ));
        };
        budget = remaining;
        let found = read_file_box(file, offset, file_len)?;
        if !found_container && !is_placeholder(&found.kind) {
            if found.kind != *b"ftyp" && !leads_iso_bmff(&found.kind) {
                return Err(Mp4MetadataError::UnsupportedContainer(sniffed(&found.kind)));
            }
            found_container = true;
        }
        if found.kind == *b"moov" {
            return Ok((found.offset, found.size));
        }
        offset += found.size;
    }
    if !found_container {
        // Too short to even hold one box header: no container here at all.
        return Err(Mp4MetadataError::UnsupportedContainer(
            "not ISO-BMFF".to_string(),
        ));
    }
    Err(Mp4MetadataError::UnsupportedContainer("moov".to_string()))
}

/// A top-level box that says nothing about the container and may sit in front
/// of its brand: the padding a writer emits (`free`, `skip`, and QuickTime's
/// `wide`). Stepping over them keeps the sniff — the one that tells an ISO-BMFF
/// file from a foreign container — aimed at the box it is about.
fn is_placeholder(kind: &[u8; 4]) -> bool {
    is_padding(kind) || *kind == *b"wide"
}

/// A top-level box a file that names no brand can legitimately lead with: the
/// movie header, or the media QuickTime's `wide` spacer sits in front of.
///
/// `ftyp` is where the ISO-BMFF family writes its brand, and a classic
/// QuickTime `.mov` has none — the movie header may come first, or the media
/// may. QuickTime is a defined brand of this very family, so a file that names
/// no brand and opens with a box of the family is this container, and refusing
/// it left `mov` in [`WRITABLE_EXTENSIONS`] naming a file neither the reader
/// nor the writer could open. The sniff is here to tell a file apart from a
/// foreign one (MKV's EBML header, AVI's `RIFF` size word), not to require a
/// box the format does not ask for; anything outside this set is still
/// refused.
fn leads_iso_bmff(kind: &[u8; 4]) -> bool {
    *kind == *b"moov" || *kind == *b"mdat"
}

/// A top-level box located by its header alone.
struct FileBox {
    offset: u64,
    size: u64,
    kind: [u8; 4],
}

/// Reads the header of the top-level box at `offset`, never its payload.
fn read_file_box(file: &mut File, offset: u64, file_len: u64) -> Result<FileBox, Mp4MetadataError> {
    // Every size comparison below is made against the bytes actually left in the
    // file, never against `offset + size`: the 64-bit `largesize` form can
    // declare `2^64 - offset`, whose sum wraps to a small value that would pass
    // an unchecked bounds check and send the scan in circles.
    if offset > file_len || file_len - offset < 8 {
        return Err(Mp4MetadataError::UnsupportedContainer(
            "not ISO-BMFF".to_string(),
        ));
    }
    let available = file_len - offset;
    file.seek(SeekFrom::Start(offset))
        .map_err(Mp4MetadataError::Io)?;
    let mut header = [0u8; 8];
    file.read_exact(&mut header).map_err(truncated_container)?;
    let mut kind = [0u8; 4];
    kind.copy_from_slice(&header[4..8]);
    let (size, header_len) = match u32::from_be_bytes([header[0], header[1], header[2], header[3]])
    {
        0 => (available, 8),
        1 if available >= 16 => {
            let mut large = [0u8; 8];
            file.read_exact(&mut large).map_err(truncated_container)?;
            (u64::from_be_bytes(large), 16)
        }
        // Also covers `size == 1` with a truncated `largesize`: 1 is below the
        // 8-byte header floor, so the guard below rejects it.
        declared => (u64::from(declared), 8),
    };
    if size < header_len || size > available {
        return Err(Mp4MetadataError::UnsupportedContainer(sniffed(&kind)));
    }
    Ok(FileBox { offset, size, kind })
}

/// A header that runs off the end of the file is a truncated container, not an
/// I/O failure of the filesystem.
fn truncated_container(err: std::io::Error) -> Mp4MetadataError {
    match err.kind() {
        ErrorKind::UnexpectedEof => {
            Mp4MetadataError::UnsupportedContainer("not ISO-BMFF".to_string())
        }
        _ => Mp4MetadataError::Io(err),
    }
}

/// Names what was found where an ISO-BMFF box was expected: the four-character
/// type when it is readable, otherwise a plain verdict. A binary type field —
/// an AVI `RIFF` size word, an EBML id — is never a container this reader can
/// make sense of, so the message says so instead of printing mojibake.
fn sniffed(kind: &[u8; 4]) -> String {
    if kind.iter().all(|byte| byte.is_ascii_graphic()) {
        String::from_utf8_lossy(kind).into_owned()
    } else {
        "not ISO-BMFF".to_string()
    }
}

/// Reads the payload of the `moov` box located by [`locate_moov`].
fn read_moov(file: &mut File, offset: u64, size: u64) -> Result<Vec<u8>, Mp4MetadataError> {
    if size > MAX_MOOV_BYTES {
        return Err(Mp4MetadataError::NoRoom("moov"));
    }
    let size = usize::try_from(size).map_err(|_| Mp4MetadataError::NoRoom("moov"))?;
    let mut buf = vec![0u8; size];
    file.seek(SeekFrom::Start(offset))
        .map_err(Mp4MetadataError::Io)?;
    file.read_exact(&mut buf).map_err(truncated_container)?;
    Ok(buf)
}

/// First direct child of `parent` with the given type.
fn find_child<'a>(parent: &'a BoxSpan, kind: &[u8; 4]) -> Option<&'a BoxSpan> {
    parent.children.iter().find(|child| child.kind == *kind)
}

/// The box reached by following `path` down from `root`.
fn find_path<'a>(root: &'a BoxSpan, path: &[[u8; 4]]) -> Option<&'a BoxSpan> {
    let mut current = root;
    for kind in path {
        current = find_child(current, kind)?;
    }
    Some(current)
}

/// Whether `kind` occurs anywhere at or below `span`.
fn contains_kind(span: &BoxSpan, kind: &[u8; 4]) -> bool {
    span.kind == *kind || span.children.iter().any(|child| contains_kind(child, kind))
}

/// `mvhd` creation time, decoded from the QuickTime epoch.
///
/// A stored 0 means "never set" — ffprobe reports no `creation_time` for it —
/// so it is answered as `None` rather than as 1904-01-01, even though 0 decodes
/// perfectly well.
fn mvhd_creation_time(
    buf: &[u8],
    moov: &BoxSpan,
) -> Result<Option<DateTime<Utc>>, Mp4MetadataError> {
    let Some(mvhd) = find_child(moov, b"mvhd") else {
        return Ok(None);
    };
    let body = mvhd.offset + header_len(buf, mvhd.offset);
    let end = mvhd.offset + mvhd.size;
    // version byte, then the creation field: 32-bit for version 0, 64-bit for 1.
    let version = *buf.get(body).ok_or(Mp4MetadataError::InvalidDate)?;
    let width = if version == 1 { 8 } else { 4 };
    let field_end = body + 4 + width;
    if field_end > end {
        return Err(Mp4MetadataError::InvalidDate);
    }
    let field = &buf[body + 4..field_end];
    let stored = if width == 8 {
        u64::from_be_bytes(<[u8; 8]>::try_from(field).map_err(|_| Mp4MetadataError::InvalidDate)?)
    } else {
        u64::from(u32::from_be_bytes(
            <[u8; 4]>::try_from(field).map_err(|_| Mp4MetadataError::InvalidDate)?,
        ))
    };
    if stored == 0 {
        return Ok(None);
    }
    let unix_seconds = i64::try_from(stored).map_err(|_| Mp4MetadataError::InvalidDate)?
        - QUICKTIME_EPOCH_UNIX_SECONDS;
    DateTime::from_timestamp(unix_seconds, 0)
        .map(Some)
        .ok_or(Mp4MetadataError::InvalidDate)
}

/// Key names of `moov/udta/meta/keys`, in file order: the value at position `n`
/// is indexed by the item type `n + 1`.
fn mdta_keys(buf: &[u8], moov: &BoxSpan) -> Vec<String> {
    let Some(keys) = find_path(moov, &[*b"udta", META_BOX, *b"keys"]) else {
        return Vec::new();
    };
    let body = keys.offset + header_len(buf, keys.offset);
    let end = keys.offset + keys.size;
    // version/flags word, then the entry count.
    let Some(entry_count) = size_field(buf, body + 4) else {
        return Vec::new();
    };
    let mut offset = body + 8;
    // The declared count is never walked as it reads. It is reduced first to
    // what the remaining payload can hold — an entry costs at least
    // `MIN_MDTA_ENTRY_BYTES`, so nothing past that many is there to be read —
    // and then to [`MAX_MDTA_KEYS`], which bounds what the names could cost.
    let walkable = usize::try_from(entry_count)
        .unwrap_or(usize::MAX)
        .min(end.saturating_sub(offset) / MIN_MDTA_ENTRY_BYTES)
        .min(MAX_MDTA_KEYS);
    // Sized from that bound rather than grown into it: a vector doubling its
    // way to the cap spends twice what the cap costs at its peak.
    let mut names = Vec::with_capacity(walkable);
    for _ in 0..walkable {
        // Each entry is `size` (which covers itself and the namespace), the
        // 4-byte namespace, then the key bytes.
        let Some(entry_size) = size_field(buf, offset) else {
            break;
        };
        let entry_size = usize::try_from(entry_size).unwrap_or(usize::MAX);
        // `checked_add` rather than `offset + entry_size`: the declared size is
        // an attacker-controlled `u32` this reader never sizes an allocation
        // from, and on a 32-bit target the sum wraps — a wrapped bound passes
        // the check and the slice behind it panics.
        let Some(entry_end) = offset.checked_add(entry_size) else {
            break;
        };
        if entry_size < MIN_MDTA_ENTRY_BYTES || entry_end > end {
            break;
        }
        names.push(text_of(&buf[offset + MIN_MDTA_ENTRY_BYTES..entry_end]));
        offset = entry_end;
    }
    names
}

/// One item of `moov/udta/meta/ilst`.
struct ItemValue {
    /// The item type: an mdta key index or a `©`-prefixed legacy type.
    kind: [u8; 4],
    text: String,
}

/// Values of `moov/udta/meta/ilst`, in file order.
///
/// An item wraps a `data` box whose payload opens with a 4-byte type indicator
/// and a 4-byte locale ahead of the value.
fn ilst_items(buf: &[u8], moov: &BoxSpan) -> Vec<ItemValue> {
    let Some(ilst) = find_path(moov, &[*b"udta", META_BOX, *b"ilst"]) else {
        return Vec::new();
    };
    ilst.children
        .iter()
        .filter_map(|item| {
            let data = find_child(item, b"data")?;
            let start = data.offset + header_len(buf, data.offset) + 8;
            let end = data.offset + data.size;
            if start > end {
                return None;
            }
            Some(ItemValue {
                kind: item.kind,
                text: text_of(&buf[start..end]),
            })
        })
        .collect()
}

/// The `ilst` values a container's mdta carriers carry, indexed by the 1-based
/// key position their item type names.
///
/// Built in one pass over the item list and read by hash probe afterwards, so
/// a lookup costs the key list rather than the key list times the item list.
/// Both are individually capped ([`MAX_MDTA_KEYS`], and the item list by
/// [`MAX_BOXES`]), which bounds no product at all: a container naming one key
/// in every position and filling every position with an item costs billions of
/// comparisons on every read of every video on every scan, and the two
/// megabytes it takes to build one are nothing.
struct MdtaIndex {
    by_index: HashMap<u32, String>,
}

// How many `ilst` items each path has taken into an index, per thread.
//
// The counts are on the passes, not on a clock: a wall clock says the work
// finished, not that the item list was read once, and an index a test builds
// for itself says nothing about the index the code under test builds. They are
// thread-local so tests running in parallel never share a count, and separate
// because the two paths count different things — a read indexes the values it
// finds, while a save indexes the carriers it resolves, and only the second
// says whether the resolution went through the index at all.
#[cfg(test)]
thread_local! {
    static ITEMS_WALKED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static CARRIERS_RESOLVED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl MdtaIndex {
    /// Indexes `items` under the key position each of them names.
    ///
    /// The first item of a position that holds text wins, and an item holding
    /// an all-NUL payload does not take the position away from a later item
    /// that does fill it — the rule [`legacy_item_value`] applies, and the one
    /// the write side walks them under.
    fn of(items: &[ItemValue]) -> Self {
        let mut by_index = HashMap::new();
        for item in items {
            // The pass itself, counted where it happens: a lookup that went
            // back to rescanning the items per named position would never come
            // through here, and would leave the count at zero while still
            // reading the right value.
            #[cfg(test)]
            ITEMS_WALKED.with(|walked| walked.set(walked.get() + 1));
            // Only a `0`-prefixed item type is an index; a `©`-prefixed legacy
            // type is not one and is read by its own name.
            if item.kind[0] != 0 || item.text.is_empty() {
                continue;
            }
            by_index
                .entry(u32::from_be_bytes(item.kind))
                .or_insert_with(|| item.text.clone());
        }
        Self { by_index }
    }

    /// The value the item at 1-based `position` carries.
    fn get(&self, position: u32) -> Option<&str> {
        self.by_index.get(&position).map(String::as_str)
    }
}

/// The `ilst` items of a `moov`, indexed by the item type each of them
/// carries — the writer's counterpart to [`MdtaIndex`], built in the same one
/// pass and read by hash probe afterwards.
///
/// Resolving a carrier used to cost the key list times the item list: both
/// collectors walked every named key position and rescanned the whole item
/// list for each. Neither factor is bounded by anything but the file itself —
/// [`MAX_MDTA_KEYS`] key positions against an item list capped only by
/// [`MAX_BOXES`] — so a container naming one key in every position and filling
/// every position with an item turned a single save into billions of
/// item/type comparisons, twice over: once for the date and once for the
/// position, and again for every undo, which renders the same region a second
/// time to check what the first one wrote.
///
/// Nothing about which carriers a save targets changes: every position a key
/// names is still resolved (a key named twice keeps both its items in step), a
/// position no item carries is still skipped, and the items of one type are
/// still walked in the order the item list holds them, so the first carrier
/// that cannot hold a value is still the one a refusal names.
struct IlstIndex<'a> {
    by_kind: HashMap<[u8; 4], Vec<&'a BoxSpan>>,
}

impl<'a> IlstIndex<'a> {
    /// Indexes the items of `moov`'s `ilst`; an empty index when it holds none.
    fn of(moov: &'a BoxSpan) -> Self {
        let Some(ilst) = find_path(moov, &[*b"udta", META_BOX, *b"ilst"]) else {
            return Self {
                by_kind: HashMap::new(),
            };
        };
        let mut by_kind: HashMap<[u8; 4], Vec<&BoxSpan>> = HashMap::new();
        for item in &ilst.children {
            by_kind.entry(item.kind).or_default().push(item);
        }
        Self { by_kind }
    }

    /// Whether the file carries an `ilst` at all.
    ///
    /// A file whose `meta` holds `keys` but no `ilst` names every key in vain,
    /// and reading that name list is the expensive half of resolving a
    /// carrier — so the collectors skip it entirely rather than build a list no
    /// item can answer.
    fn is_empty(&self) -> bool {
        self.by_kind.is_empty()
    }

    /// The items carrying `kind`, in the order the item list holds them.
    ///
    /// The only door to the items, which is what the count is measured at: a
    /// collector that went back to walking the item list itself never comes
    /// through here and so resolves the same carriers while counting none —
    /// where a save through the index counts each item it was handed once, no
    /// matter how many positions name it.
    fn items(&self, kind: [u8; 4]) -> &[&'a BoxSpan] {
        let found = self.by_kind.get(&kind).map_or(&[][..], Vec::as_slice);
        #[cfg(test)]
        CARRIERS_RESOLVED.with(|resolved| resolved.set(resolved.get() + found.len()));
        found
    }
}

/// First mdta value among `names` (in the order given) that the file carries.
///
/// An item holding empty text is not a hit: a file can name a key and store an
/// all-NUL payload under it, which reads as no text at all. The emptiness is
/// part of the lookup rather than a filter applied afterwards, so a file that
/// carries a kind twice — the first item empty, the second holding the value —
/// still answers with the later item, the way the write side walks them.
///
/// Every position at which `keys` names `name` is consulted, not only the
/// first: a file that names a key twice is carried by an item per position, and
/// the writer patches every one of them, so the reader has to see them all.
fn mdta_value(index: &MdtaIndex, keys: &[String], names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        keys.iter()
            .enumerate()
            .filter(|(_, key)| key.as_str() == *name)
            .find_map(|(position, _)| {
                // mdta indexes are 1-based.
                let key = u32::try_from(position + 1).ok()?;
                index.get(key)
            })
            .map(str::to_string)
    })
}

/// First value stored under the legacy `kind` item type that holds text.
///
/// The same rule as [`mdta_value`]: an item holding empty text is not a hit, so
/// a file carrying `©xyz` (or `©day`) twice — the first item all-NUL, the second
/// holding the value — answers with the later item, the way the write side
/// walks every item of the kind.
fn legacy_item_value(items: &[ItemValue], kind: &[u8; 4]) -> Option<String> {
    items
        .iter()
        .find(|item| item.kind == *kind && !item.text.is_empty())
        .map(|item| item.text.clone())
}

/// Value of a direct `moov/udta/<kind>` text box — the pre-`meta` QuickTime
/// home of `©day` and `©xyz`.
///
/// A container may hold the same child twice. Every one of them is consulted,
/// the way the `ilst` items of the same value are, and the first that holds text
/// wins: an empty duplicate is no more a carrier than an empty `data` payload
/// is, and the writer rewrites all of them, so a reader that stopped at the
/// first would keep answering with a value the last save replaced.
fn direct_udta_value(buf: &[u8], moov: &BoxSpan, kind: &[u8; 4]) -> Option<String> {
    direct_udta_children(moov, kind)
        .filter_map(|child| {
            let (_, text) = direct_text_slot(buf, child);
            buf.get(text).map(text_of)
        })
        .find(|text| !text.is_empty())
}

/// Every direct `moov/udta` child of the given type, in file order — the read
/// and the write side walk the same list, so every position the reader can
/// resolve is a position the writer patches.
fn direct_udta_children<'a>(
    moov: &'a BoxSpan,
    kind: &[u8; 4],
) -> impl Iterator<Item = &'a BoxSpan> {
    let kind = *kind;
    find_child(moov, b"udta")
        .into_iter()
        .flat_map(move |udta| udta.children.iter().filter(move |child| child.kind == kind))
}

/// The byte ranges of a direct `moov/udta/<kind>` child's text: the box payload
/// (text-atom header included) and the text inside it.
///
/// Classic QuickTime writes these children as *text atoms*: a `u16` byte count
/// and a `u16` language code in front of the text — ffmpeg stores a position as
/// `00 12 55 c4 +48.2082+016.3737/`, and the scan-time `moov` remux produces the
/// same shape. Some writers put the text straight into the payload instead, so
/// the two layouts are told apart by the leading word, which a text atom sets
/// to exactly the bytes behind its 4-byte header — terminator and NUL padding
/// included. The header is never part of the value; it is part of the payload a
/// rewrite replaces, so a rewritten atom carries its count along, and a count
/// that no longer spans the whole payload stops naming this layout for the next
/// read ([`direct_text_edit`]).
fn direct_text_slot(buf: &[u8], direct: &BoxSpan) -> (Range<usize>, Range<usize>) {
    let payload = direct.offset + header_len(buf, direct.offset);
    let end = direct.offset + direct.size;
    let header = match buf.get(payload..end).and_then(|bytes| bytes.get(..2)) {
        Some(word) if usize::from(u16::from_be_bytes([word[0], word[1]])) + 4 == end - payload => 4,
        _ => 0,
    };
    (payload..end, payload + header..end)
}

/// The first carrier of a chain that holds text, in priority order.
///
/// A carrier holding an empty value — an all-NUL `data` payload reads as empty
/// text — is not a carrier: letting it win would answer "no position" for a file
/// that spells its position one level further down the chain.
fn first_non_empty(values: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    values.into_iter().flatten().find(|value| !value.is_empty())
}

/// An ISO/3GPP `udta/loci` position, as the ISO 6709 text the other carriers
/// spell, or `None` when the file has no exact `loci` position.
///
/// `loci` (ISO/IEC 14496-12 `LocationInformationBox`) is a FullBox whose payload
/// holds a version/flags word, a 2-byte language, a NUL-terminated name and a
/// role byte in front of the horizontal pair — longitude then latitude, each a
/// 16.16 fixed-point `i32`. ffmpeg's MP4 muxer writes it for a location tag, and
/// the scan-time `moov` remux (`-c copy -movflags +faststart`) hands a file
/// whose only carrier was a direct `©xyz` child to that muxer, so this is the
/// carrier such a file ends up with. What the pair holds is reported verbatim,
/// the way a text carrier's value is; validating the position is the caller's
/// business, and a box too short to hold a pair simply is not a carrier.
fn loci_location(buf: &[u8], moov: &BoxSpan) -> Option<String> {
    direct_udta_children(moov, b"loci").find_map(|loci| {
        let pair = loci_pair_offset(buf, loci)?;
        let longitude = fixed_point_value(buf.get(pair..pair + 4)?)?;
        let latitude = fixed_point_value(buf.get(pair + 4..pair + 8)?)?;
        Some(iso6709_text(latitude, longitude))
    })
}

/// Where a `loci` box's 16.16 horizontal pair starts, or `None` when the box is
/// too short for the fields in front of it or does not name an exact position.
///
/// The role byte in front of the pair says what the pair means: 0 is the place
/// itself, and 1..=8 are coarser — room, building, floor, postcode,
/// intersection, island, street, address. A pair that names a postcode or a
/// street is not a position this reader may report as one: it is a region
/// whose extent the box does not store, and a longitude/latitude read out of
/// it would place a video somewhere its own metadata never claimed. So only
/// role 0 is a carrier, on the read side and — through the same
/// [`loci_pair_offset`] — on the write side too: a file whose only location
/// is a coarse `loci` is refused the position rather than having the region
/// silently overwritten with a point.
fn loci_pair_offset(buf: &[u8], loci: &BoxSpan) -> Option<usize> {
    let body = loci.offset + header_len(buf, loci.offset);
    let end = loci.offset + loci.size;
    // The version/flags word and the language code, then a NUL-terminated name.
    let name = (body + 6).min(end);
    let name_length = buf.get(name..end)?.iter().position(|byte| *byte == 0)?;
    // The name's terminator, then the role byte, then the pair itself.
    let role = name + name_length + 1;
    let pair = role + 1;
    (buf.get(role) == Some(&LOCI_ROLE_EXACT) && pair + 8 <= end).then_some(pair)
}

/// The degrees a 16.16 fixed-point `i32` stores, or `None` when there are not
/// four bytes to read.
fn fixed_point_value(bytes: &[u8]) -> Option<f64> {
    let bytes = <[u8; 4]>::try_from(bytes).ok()?;
    Some(f64::from(i32::from_be_bytes(bytes)) / LOCI_FIXED_POINT_SCALE)
}

/// `value` as the 16.16 fixed-point `i32` a `loci` pair stores, `None` when it
/// is not finite or falls outside what the format can hold.
fn fixed_point_16_16(value: f64) -> Option<i32> {
    if !value.is_finite() {
        return None;
    }
    let scaled = (value * LOCI_FIXED_POINT_SCALE).round();
    if scaled < f64::from(i32::MIN) || scaled > f64::from(i32::MAX) {
        return None;
    }
    Some(scaled as i32)
}

/// A horizontal pair as ISO 6709 text, in the shape the other carriers and
/// ffprobe write: four decimals per field, the latitude's degrees two digits
/// wide and the longitude's three, closed by a solidus.
fn iso6709_text(latitude: f64, longitude: f64) -> String {
    format!(
        "{}{}/",
        iso6709_field(latitude, 2),
        iso6709_field(longitude, 3)
    )
}

/// One field of [`iso6709_text`]: an explicit sign, the degrees zero-padded to
/// `degrees_width`, and four decimals of the fraction.
///
/// The fraction is taken from the scaled integer rather than from a rounded
/// float, so a value whose fraction rounds up to a whole degree carries into
/// the degrees instead of being written as `.10000`.
fn iso6709_field(value: f64, degrees_width: usize) -> String {
    let scaled = (value.abs() * 10_000.0).round() as u64;
    let (degrees, fraction) = (scaled / 10_000, scaled % 10_000);
    let sign = if value.is_sign_negative() && scaled != 0 {
        '-'
    } else {
        '+'
    };
    format!(
        "{sign}{degrees:0width$}.{fraction:04}",
        width = degrees_width
    )
}

/// The date text and location text the container carries, each from the first
/// carrier that holds one: mdta keys, then the legacy item type, then a direct
/// `udta` child, then — for a location — an ISO/3GPP `udta/loci` pair.
///
/// The carrier list is the writer's, too: [`collect_text_date_edits`] and
/// [`collect_location_edits`] target every carrier consulted here — each
/// position a `keys` entry names the key at, each item of a legacy type, the
/// direct children and the `loci` pair — so a value a save lands in is always a
/// value this reader resolves back, and an empty slot is skipped on both sides
/// alike.
fn text_carriers(buf: &[u8], moov: &BoxSpan) -> (Option<String>, Option<String>) {
    let keys = mdta_keys(buf, moov);
    let items = ilst_items(buf, moov);
    // Indexed once for both lookups below: the item list is the one that is
    // worth walking once.
    let index = MdtaIndex::of(&items);
    let date = first_non_empty([
        mdta_value(&index, &keys, &MDTA_CREATION_DATE_KEYS),
        legacy_item_value(&items, &DAY_BOX),
        direct_udta_value(buf, moov, &DAY_BOX),
    ]);
    let location = first_non_empty([
        mdta_value(&index, &keys, &MDTA_LOCATION_KEYS),
        legacy_item_value(&items, &XYZ_BOX),
        direct_udta_value(buf, moov, &XYZ_BOX),
        loci_location(buf, moov),
    ]);
    (date, location)
}

/// Box text: UTF-8, with trailing NUL bytes stripped. Those bytes are the
/// padding a rewrite leaves behind when its rendering is shorter than the slot
/// it fills (ffmpeg writes none, Apple writes one), never part of the value. No
/// other whitespace is touched — the stored value has to survive a rewrite
/// verbatim.
fn text_of(bytes: &[u8]) -> String {
    String::from_utf8_lossy(trim_trailing_nuls(bytes)).into_owned()
}

/// `bytes` without its trailing NUL padding; a payload that is all NULs is
/// empty text.
fn trim_trailing_nuls(bytes: &[u8]) -> &[u8] {
    let end = bytes
        .iter()
        .rposition(|byte| *byte != 0)
        .map_or(0, |last| last + 1);
    &bytes[..end]
}

/// Latitude and longitude of an ISO 6709 string, `None` when it is not one.
///
/// Accepts the forms QuickTime writes — `±DD.D±DDD.D[±DDD.D][/]`, altitude and
/// the closing solidus optional — and validates the horizontal pair
/// (`-90..=90` / `-180..=180`). The altitude is parsed so that it cannot be
/// mistaken for input the format does not have, then dropped.
pub fn parse_iso6709(value: &str) -> Option<(f64, f64)> {
    let value = value.strip_suffix('/').unwrap_or(value);
    let bytes = value.as_bytes();
    let mut components = [0.0f64; 3];
    let mut count = 0;
    let mut index = 0;
    while index < bytes.len() && count < 3 {
        let start = index;
        if !matches!(bytes[index], b'+' | b'-') {
            return None;
        }
        index += 1;
        let digits = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index == digits {
            return None;
        }
        if bytes.get(index) == Some(&b'.') {
            index += 1;
            let fraction = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            if index == fraction {
                return None;
            }
        }
        components[count] = value.get(start..index)?.parse().ok()?;
        count += 1;
    }
    if index != bytes.len() || count < 2 {
        return None;
    }
    let (latitude, longitude) = (components[0], components[1]);
    if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
        return None;
    }
    Some((latitude, longitude))
}

/// What to change about a container's recording metadata.
///
/// `taken_at` is the instant itself; `latitude`/`longitude` are the horizontal
/// pair of an ISO 6709 position, which is only valid as a pair.
#[derive(Debug, Clone, Copy, Default)]
pub struct VideoMetadataEdit {
    pub taken_at: Option<DateTime<Utc>>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

/// What a successful write produced: the file's new identity as the scanner
/// records it, and how to put it back.
#[derive(Debug)]
pub struct VideoMetadataWrite {
    pub fingerprint: Fingerprint,
    pub undo: UndoToken,
}

/// Identity of a file after a write, in exactly the form the scanner stores and
/// compares: byte length plus modification time truncated to whole seconds
/// (`src/file_scanner.rs`, `src/db.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fingerprint {
    pub file_size: u64,
    pub file_modified: DateTime<Utc>,
}

/// Everything needed to put the file back the way it was: which `moov` region
/// was replaced, the bytes that were there, and the modification time to
/// restore.
///
/// The path is part of the token because [`restore`] is called from a failure
/// path that has nothing else about the file at hand.
#[derive(Debug)]
pub struct UndoToken {
    path: PathBuf,
    moov_offset: u64,
    moov_bytes: Vec<u8>,
    /// The edit the region was written with. [`restore_region`] renders the
    /// region from the bytes above and this again, and only writes the bytes
    /// back over a file that still holds what that rendering produced — the
    /// undo checks the region by replaying the save, not by recognising the
    /// shape of a box header.
    edit: VideoMetadataEdit,
    modified: SystemTime,
    /// The file's byte length when the region was written: the region only fits
    /// the file it was measured in, so a file of another length is not the one
    /// this token may put back.
    file_len: u64,
}

/// Earliest instant the writer accepts, as Unix seconds. The reader rejects
/// anything before 1990 (`src/metadata_extractor.rs`), so storing one would
/// hand back a file the reader refuses.
const MIN_WRITABLE_UNIX_SECONDS: i64 = 631_152_000; // 1990-01-01T00:00:00Z

/// Latest instant the writer accepts, as Unix seconds: the largest version-0
/// QuickTime timestamp, `u32::MAX` seconds after 1904-01-01T00:00:00Z.
const MAX_WRITABLE_UNIX_SECONDS: i64 = 2_212_122_495; // 2040-02-06T06:28:15Z

/// FullBoxes whose payload opens with a creation (and modification) time.
const TIME_BOXES: [[u8; 4]; 3] = [*b"mvhd", *b"tkhd", *b"mdhd"];

/// One region of the `moov` a carrier replaces, and what replaces it.
///
/// A rendering is padded with NULs to the payload slot it replaces, so a value
/// that did not grow leaves the container's length alone; one that did grow is
/// paid for out of the file's own padding ([`rebuild_with_padding`]).
struct MoovEdit {
    /// Offset of the region in the original `moov`.
    offset: usize,
    /// Length of the region in the original `moov`.
    replaced: usize,
    /// What replaces it.
    content: Vec<u8>,
    /// The carrier this region belongs to, as a failure names it.
    carrier: &'static str,
}

impl MoovEdit {
    /// Bytes this region adds (positive) or removes (negative).
    fn delta(&self) -> isize {
        self.content.len() as isize - self.replaced as isize
    }
}

/// Rewrites a container's recording metadata: the creation timestamps of
/// `mvhd`, `tkhd` and `mdhd`, every text date carrier the file holds, and every
/// location carrier. Hands back a fingerprint plus an undo token.
///
/// Each carrier is rendered in its own representation (see
/// [`render_date_in_shape`] and [`render_iso6709_in_shape`]), so the carriers
/// still name one instant and one position after the save. A rendering that
/// needs more bytes than its slot holds is paid for out of the file's own
/// padding inside the `moov` (see [`rebuild_with_padding`]); when the file has
/// no room for it, the save is refused. The file's byte length is unchanged
/// either way.
///
/// All-or-nothing: the date and the coordinates are validated, the `moov` tree
/// is parsed and the whole replacement is rendered and length-checked before
/// the file is opened for writing, and the write itself is a single `write_all`
/// of the `moov` region, which is verified against the bytes it was measured
/// from, put back if it fails, and given its modification time back either way
/// ([`write_moov_region`]). No byte outside `moov` is ever touched, and the file
/// is never truncated or grown.
///
/// An instant no box in the container carries is refused, the way a position
/// without a carrier is ([`Mp4MetadataError::NoLocationCarrier`]): a save that
/// rendered no date carrier would report an instant the file does not hold. An
/// edit that names no value at all is answered from the file as it is — the
/// replacement is still rendered and checked, and no file is opened for writing.
pub fn write_metadata(
    path: &Path,
    edit: &VideoMetadataEdit,
) -> Result<VideoMetadataWrite, Mp4MetadataError> {
    if !has_writable_extension(path) {
        return Err(Mp4MetadataError::UnsupportedContainer(extension_name(path)));
    }
    validate_edit(edit)?;

    let mut source = File::open(path).map_err(open_error)?;
    let original_metadata = source.metadata().map_err(Mp4MetadataError::Io)?;
    let original_modified = original_metadata.modified().map_err(Mp4MetadataError::Io)?;
    // The fingerprint this save hands back is the file's own clock, and the
    // write hands that same clock back afterwards. A file whose clock predates
    // the Unix epoch has none the scanner can store — `file_scanner` reports
    // no modification time at all for it — so there is no honest fingerprint
    // to hand back, and an invented wall-clock instant would mismatch the row
    // on every later scan, re-extracting a file this save never changed. The
    // save is refused here, before a byte is touched, rather than answered with
    // a value nothing can reproduce.
    if truncate_to_seconds(original_modified).is_none() {
        return Err(Mp4MetadataError::Unrepresentable("modification time"));
    }
    let (moov_offset, moov_size) = locate_moov(&mut source, original_metadata.len())?;
    let original_moov = read_moov(&mut source, moov_offset, moov_size)?;
    drop(source);
    let patched = patch_moov(&original_moov, edit)?;

    // An edit that names no value is answered from the file as it is: the
    // replacement is still rendered and length-checked above, so a container
    // that cannot hold it is refused as it is for a real edit, but the file is
    // never opened for writing.
    if requests_a_write(edit) {
        let mut target = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(open_error)?;
        write_moov_region(
            &mut target,
            moov_offset,
            &original_moov,
            &patched,
            original_modified,
            path,
        )?;
        drop(target);
    }

    let written = std::fs::metadata(path).map_err(Mp4MetadataError::Io)?;
    Ok(VideoMetadataWrite {
        fingerprint: Fingerprint {
            file_size: written.len(),
            // The clock the guard above accepted, read back from the file the
            // write closed: `None` now would mean the filesystem could not
            // store what it stored a moment ago, which is not a fingerprint
            // this can report either.
            file_modified: truncate_to_seconds(written.modified().map_err(Mp4MetadataError::Io)?)
                .ok_or(Mp4MetadataError::Unrepresentable("modification time"))?,
        },
        undo: UndoToken {
            path: path.to_path_buf(),
            moov_offset,
            moov_bytes: original_moov,
            edit: *edit,
            modified: original_modified,
            file_len: original_metadata.len(),
        },
    })
}

/// True when `edit` asks for a value to be written at all.
///
/// A request naming no instant and no position has nothing to render, so
/// writing the region back byte for byte would only open a file the caller
/// wanted left alone — and refuse a save it had every reason to accept on a
/// read-only medium.
fn requests_a_write(edit: &VideoMetadataEdit) -> bool {
    edit.taken_at.is_some() || edit.latitude.is_some() || edit.longitude.is_some()
}

/// Bytes of the `moov` region compared at a time when the region is read back
/// before it is replaced: the check must not cost a buffer the size of the
/// region it verifies.
const VERIFY_CHUNK_BYTES: usize = 8 * 1024;

/// A target a `moov` region is written over, and whose modification time can be
/// put back afterwards.
///
/// The region write is generic so a test can drive one that fails part-way; the
/// modification time has to go back on the file whether the region landed or
/// not, which is what this adds to `Read + Write + Seek`.
trait RegionTarget: Read + Write + Seek {
    /// Puts `time` on the file the region lives in.
    fn restore_modified(&mut self, time: SystemTime) -> std::io::Result<()>;
}

impl RegionTarget for File {
    fn restore_modified(&mut self, time: SystemTime) -> std::io::Result<()> {
        self.set_modified(time)
    }
}

/// Replaces the `moov` region of an open file with `patched`, after checking
/// that the region still holds `original`.
///
/// Both guards are about the window between reading the file and writing it.
/// The bytes being written were measured against the file that was parsed, so
/// the region is read back and compared before it is replaced: a path whose
/// file was swapped in the meantime — the indexing pass renaming a faststart
/// remux over it, say — must not get the old `moov` written at offsets that now
/// point into `mdat`. And a write that fails part-way (a full disk, an I/O
/// error, a killed process) leaves a mix of old and new bytes that no longer
/// parses, so `original` is written back over it before the error is returned:
/// a failed save leaves the file byte-identical (FR-006).
///
/// The modification time is put back on both outcomes that leave the file
/// holding what it held: a failed write whose rollback succeeded still left the
/// file looking newer than it is, and the change detection (and the
/// thumbnail/transcode cache names) key on size and modification time — the next
/// scan would re-extract, and for a `moov`-at-end source re-remux, a file that
/// did not change.
///
/// A rollback that fails too is the one outcome the file does not survive
/// intact. Its modification time then stays where the failed write left it, and
/// that is deliberate: the write moved the clock, so the fingerprint the next
/// scan keys on is a new one, and a file it now re-extracts is a file it can
/// repair. Restoring the time would hand back a file that looks untouched and
/// is not. The error says so, because the client is told the edit failed and
/// has to know the file behind it is in an unknown state.
fn write_moov_region<W: RegionTarget>(
    target: &mut W,
    moov_offset: u64,
    original: &[u8],
    patched: &[u8],
    modified: SystemTime,
    path: &Path,
) -> Result<(), Mp4MetadataError> {
    // Only a region of the length it was measured in can be written over it:
    // anything longer would push every byte behind it along. The caller
    // rejects a mismatched patch before it gets here, but the invariant
    // belongs where the write happens — a second caller must not be able to
    // break it silently, and this is the one line that would move media bytes.
    if patched.len() != original.len() {
        return Err(Mp4MetadataError::NoRoom("moov"));
    }
    let mut current = [0u8; VERIFY_CHUNK_BYTES];
    let mut offset = 0usize;
    target
        .seek(SeekFrom::Start(moov_offset))
        .map_err(Mp4MetadataError::Io)?;
    while offset < original.len() {
        let end = (offset + VERIFY_CHUNK_BYTES).min(original.len());
        let window = &mut current[..end - offset];
        target.read_exact(window).map_err(|err| match err.kind() {
            // The file is shorter than the region that was measured in it.
            ErrorKind::UnexpectedEof => Mp4MetadataError::NoRoom("moov"),
            _ => Mp4MetadataError::Io(err),
        })?;
        if window != &original[offset..end] {
            return Err(Mp4MetadataError::NoRoom("moov"));
        }
        offset = end;
    }
    target
        .seek(SeekFrom::Start(moov_offset))
        .map_err(Mp4MetadataError::Io)?;
    let err = match target.write_all(patched) {
        Ok(()) => {
            if let Err(err) = target.restore_modified(modified) {
                log::warn!(
                    "could not restore the modification time of {}: {err}",
                    path.display()
                );
            }
            return Ok(());
        }
        Err(err) => err,
    };
    // Best effort: the device that just refused a write may refuse this one
    // too, and there is nothing left to do but to make that loud.
    let rollback = target
        .seek(SeekFrom::Start(moov_offset))
        .and_then(|_| target.write_all(original));
    if let Err(undo) = &rollback {
        log::error!(
            "could not put the moov region of a failed write back ({undo}); {} may be damaged and \
             will be re-examined on the next scan",
            path.display()
        );
        return Err(Mp4MetadataError::Io(unrepaired(err, undo)));
    }
    if let Err(err) = target.restore_modified(modified) {
        log::warn!(
            "could not restore the modification time of {}: {err}",
            path.display()
        );
    }
    Err(Mp4MetadataError::Io(err))
}

/// The I/O error of a write whose rollback the same device refused, with the
/// outcome spelled out: the region is a mix of old and new bytes that nothing
/// put back, and the modification time was left moved so the next scan sees a
/// file that changed.
fn unrepaired(err: std::io::Error, rollback: &std::io::Error) -> std::io::Error {
    std::io::Error::new(
        err.kind(),
        format!(
            "{err}; the moov region could not be put back ({rollback}) and the file is damaged"
        ),
    )
}

/// Puts the `moov` region and the modification time back as the token recorded
/// them.
///
/// Refuses to write when the file is no longer the one the token describes: a
/// different length, or a modification time that moved since the write, means
/// the region the token names is not this file's any more, and grafting the old
/// `moov` onto it would damage whatever is there now.
pub fn restore(token: &UndoToken) -> Result<(), Mp4MetadataError> {
    let mut target = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&token.path)
        .map_err(open_error)?;
    let current = target.metadata().map_err(Mp4MetadataError::Io)?;
    // Whole seconds, the way this module's own identity is stated everywhere
    // else: the write handed the file's timestamp back to the filesystem as it
    // came off `stat`, and a filesystem is free to store only what its format
    // holds — FAT32 in two seconds, HFS+ in one, exFAT in two — so a `stat`
    // that reads back a value the write never had is a statement about the
    // device, not about the file. Comparing sub-second precision would refuse
    // the undo of every write to a FAT32 or exFAT volume, leaving the patched
    // file in place with no way back. A clock that moved by a whole second or
    // more is still refused: that is a real edit between the write and here.
    if current.len() != token.file_len
        || truncate_to_seconds(current.modified().map_err(Mp4MetadataError::Io)?)
            != truncate_to_seconds(token.modified)
    {
        return Err(Mp4MetadataError::NoRoom("moov"));
    }
    let end = token
        .moov_offset
        .checked_add(token.moov_bytes.len() as u64)
        .ok_or(Mp4MetadataError::NoRoom("moov"))?;
    if end > current.len() {
        return Err(Mp4MetadataError::NoRoom("moov"));
    }
    restore_region(&mut target, token)
}

/// The undo write itself, over a target a test can make fail part-way — the
/// split [`write_moov_region`] has.
///
/// The content guard is the one thing the file's own metadata cannot say: the
/// caller re-arms the modification time before it calls here (the writer's own
/// `set_modified` may have been refused), so a matching clock and length prove
/// the file is the size the write left it, not that the region at that offset
/// is still the one the write produced. What answers that is the write itself:
/// the region is rendered from the token's original bytes and its edit again,
/// and the bytes found in the file have to be exactly that. So a region that
/// changed is refused, and the check never looks at a box header — the three
/// ways a `moov` may spell its own size (32-bit, the size-0 "runs to the end"
/// form, and the 64-bit `largesize` form) all pass, because none of them
/// changes a single rendered byte.
///
/// The bytes read are the ones a write that fails part-way is rolled back to —
/// a half-restored region is a file nothing can parse, and the file is left
/// holding exactly the patched bytes that were there before the undo started.
fn restore_region<W: RegionTarget>(
    target: &mut W,
    token: &UndoToken,
) -> Result<(), Mp4MetadataError> {
    // What the write put in this region, rendered again. A pure function of
    // the token's two halves, so this is the write's own answer rather than a
    // second opinion about it — and an exact one: there is no digest to
    // collide and no field to fall out of date.
    let expected = patch_moov(&token.moov_bytes, &token.edit).map_err(|err| {
        log::error!(
            "the moov region of {} could not be rendered again for the undo ({err}); it is left \
             as it is",
            token.path.display()
        );
        err
    })?;
    // The whole region, not the 8 KiB windows the patch verifies through: this
    // copy is the rollback source, so it cannot be a stream. It is bounded by
    // the [`MAX_MOOV_BYTES`] the token's own copy of the region already is, and
    // it is only ever taken on the way to a refusal or a rollback.
    let mut current = vec![0u8; token.moov_bytes.len()];
    target
        .seek(SeekFrom::Start(token.moov_offset))
        .map_err(Mp4MetadataError::Io)?;
    target
        .read_exact(&mut current)
        .map_err(|err| match err.kind() {
            // The file is shorter than the region that was measured in it.
            ErrorKind::UnexpectedEof => Mp4MetadataError::NoRoom("moov"),
            _ => Mp4MetadataError::Io(err),
        })?;
    if current != expected {
        return Err(Mp4MetadataError::NoRoom("moov"));
    }
    target
        .seek(SeekFrom::Start(token.moov_offset))
        .map_err(Mp4MetadataError::Io)?;
    let err = match target.write_all(&token.moov_bytes) {
        Ok(()) => {
            if let Err(err) = target.restore_modified(token.modified) {
                log::warn!(
                    "could not restore the modification time of {}: {err}",
                    token.path.display()
                );
            }
            return Ok(());
        }
        Err(err) => err,
    };
    // Best effort, exactly as a failed patch is rolled back: the device that
    // just refused a write may refuse this one too, and then there is nothing
    // left to do but to make that loud.
    let rollback = target
        .seek(SeekFrom::Start(token.moov_offset))
        .and_then(|_| target.write_all(&current));
    if let Err(undo) = &rollback {
        log::error!(
            "could not put the moov region of a failed undo back ({undo}); {} may be damaged and \
             will be re-examined on the next scan",
            token.path.display()
        );
        return Err(Mp4MetadataError::Io(unrepaired(err, undo)));
    }
    if let Err(err) = target.restore_modified(token.modified) {
        log::warn!(
            "could not restore the modification time of {}: {err}",
            token.path.display()
        );
    }
    Err(Mp4MetadataError::Io(err))
}

/// Rejects an edit that could not be written before a byte is touched.
fn validate_edit(edit: &VideoMetadataEdit) -> Result<(), Mp4MetadataError> {
    if let Some(taken_at) = edit.taken_at {
        if !(MIN_WRITABLE_UNIX_SECONDS..=MAX_WRITABLE_UNIX_SECONDS).contains(&taken_at.timestamp())
        {
            return Err(Mp4MetadataError::InvalidDate);
        }
    }
    // The same rules the image writer applies (`src/metadata_writer.rs`): both
    // coordinates in range, or neither. A NaN fails the range check.
    match (edit.latitude, edit.longitude) {
        (Some(latitude), Some(longitude)) => {
            if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
                return Err(Mp4MetadataError::InvalidCoordinates);
            }
        }
        (Some(_), None) | (None, Some(_)) => return Err(Mp4MetadataError::InvalidCoordinates),
        (None, None) => {}
    }
    Ok(())
}

/// `dt` as seconds from the QuickTime epoch (1904-01-01T00:00:00Z), or `None`
/// when it does not fit a version-0 `u32` QuickTime timestamp.
pub(crate) fn quicktime_seconds(dt: DateTime<Utc>) -> Option<u32> {
    let seconds = dt.timestamp().checked_add(QUICKTIME_EPOCH_UNIX_SECONDS)?;
    u32::try_from(seconds).ok()
}

/// Renders `new` into the shape `existing` is written in, or `None` when
/// `existing` is not one of the shapes a date carrier uses.
///
/// The shape is kept, not replaced: the separator, the fraction width and the
/// offset style all come from `existing`, so a `Z` carrier stays `Z`, an offset
/// carrier says the same instant in that same offset, and a carrier that names
/// no offset gets the UTC wall clock. Only the four forms a carrier actually
/// holds are recognized — `YYYY-MM-DD`, optionally `[T ]hh:mm:ss`, optionally
/// `.fraction`, optionally `Z`/`±hhmm`/`±hh:mm` — and nothing else is invented.
pub(crate) fn render_date_in_shape(existing: &str, new: DateTime<Utc>) -> Option<Vec<u8>> {
    DateShape::parse(existing.as_bytes()).map(|shape| shape.render(new))
}

/// How a carrier writes an instant: which fields it has, how they are
/// separated, how many fraction digits it keeps and how it names its offset.
struct DateShape {
    /// `T` or ` ` between the date and the time; `None` for a date-only value.
    separator: Option<u8>,
    /// Digits behind the seconds, 0 when the value has no fraction.
    fraction_digits: usize,
    /// The zone the instant is rendered in and the text naming it.
    offset: OffsetStyle,
}

/// The offset of a carrier's value: where the instant is rendered, and the
/// literal text that says so (`Z`, `+0200`, `+02:00`, or nothing at all).
struct OffsetStyle {
    zone: FixedOffset,
    text: Vec<u8>,
}

impl DateShape {
    /// The shape of a stored value, or `None` when it is not a shape this
    /// writer knows.
    fn parse(value: &[u8]) -> Option<Self> {
        let year = digits(value, 0, 4)?;
        if value.get(4)? != &b'-' {
            return None;
        }
        let month = digits(value, 5, 2)?;
        if value.get(7)? != &b'-' {
            return None;
        }
        let day = digits(value, 8, 2)?;
        NaiveDate::from_ymd_opt(year as i32, month, day)?;

        let mut rest = value.get(10..)?;
        if rest.is_empty() {
            return Some(Self {
                separator: None,
                fraction_digits: 0,
                offset: OffsetStyle::parse(&[])?,
            });
        }
        let separator = *rest.first()?;
        if separator != b'T' && separator != b' ' {
            return None;
        }
        let hour = digits(rest, 1, 2)?;
        if rest.get(3)? != &b':' {
            return None;
        }
        let minute = digits(rest, 4, 2)?;
        if rest.get(6)? != &b':' {
            return None;
        }
        let second = digits(rest, 7, 2)?;
        NaiveTime::from_hms_opt(hour, minute, second)?;
        rest = rest.get(9..)?;

        let mut fraction_digits = 0;
        if rest.first() == Some(&b'.') {
            fraction_digits = rest[1..]
                .iter()
                .take_while(|byte| byte.is_ascii_digit())
                .count();
            if fraction_digits == 0 || fraction_digits > 9 {
                return None;
            }
            rest = rest.get(1 + fraction_digits..)?;
        }
        Some(Self {
            separator: Some(separator),
            fraction_digits,
            offset: OffsetStyle::parse(rest)?,
        })
    }

    /// `new` written the way this shape writes an instant.
    fn render(&self, new: DateTime<Utc>) -> Vec<u8> {
        let local = new.with_timezone(&self.offset.zone);
        let mut rendered = format!(
            "{:04}-{:02}-{:02}",
            local.year(),
            local.month(),
            local.day()
        );
        if let Some(separator) = self.separator {
            let fraction = if self.fraction_digits > 0 {
                // Truncated, never rounded: the digits are the instant's own,
                // only as many of them as this carrier keeps.
                let mut nanos = format!("{:09}", local.nanosecond());
                nanos.truncate(self.fraction_digits);
                format!(".{nanos}")
            } else {
                String::new()
            };
            rendered.push_str(&format!(
                "{}{:02}:{:02}:{:02}{fraction}",
                char::from(separator),
                local.hour(),
                local.minute(),
                local.second(),
            ));
        }
        let mut bytes = rendered.into_bytes();
        bytes.extend_from_slice(&self.offset.text);
        bytes
    }
}

impl OffsetStyle {
    /// The offset a value ends with, consuming every byte it has. Absent,
    /// `Z`, `±hhmm` and `±hh:mm` are the whole vocabulary; anything else — an
    /// out-of-range or half-written offset — makes the value unrenderable
    /// rather than being normalized into a style the file never had.
    fn parse(value: &[u8]) -> Option<Self> {
        let zone = |seconds: i32| FixedOffset::east_opt(seconds);
        if value.is_empty() {
            return Some(Self {
                zone: zone(0)?,
                text: Vec::new(),
            });
        }
        if value == b"Z" {
            return Some(Self {
                zone: zone(0)?,
                text: b"Z".to_vec(),
            });
        }
        let sign = match value.first()? {
            b'+' => 1i32,
            b'-' => -1i32,
            _ => return None,
        };
        let (hours, minutes) = match value.len() {
            5 => (digits(value, 1, 2)?, digits(value, 3, 2)?),
            6 if value.get(3) == Some(&b':') => (digits(value, 1, 2)?, digits(value, 4, 2)?),
            _ => return None,
        };
        if hours > 23 || minutes > 59 {
            return None;
        }
        let seconds =
            sign * (i32::try_from(hours).ok()? * 3600 + i32::try_from(minutes).ok()? * 60);
        Some(Self {
            zone: zone(seconds)?,
            text: value.to_vec(),
        })
    }
}

/// `len` decimal digits at `start` of `bytes`, as a number; `None` when any of
/// them is not a digit.
fn digits(bytes: &[u8], start: usize, len: usize) -> Option<u32> {
    let field = bytes.get(start..start + len)?;
    field.iter().try_fold(0u32, |value, byte| {
        byte.is_ascii_digit()
            .then(|| value * 10 + u32::from(byte - b'0'))
    })
}

/// Renders `latitude`/`longitude` into the position shape `existing` is written
/// in, or `None` when `existing` is not a position whose shape can be read.
///
/// The shape is kept, not replaced: each field keeps its decimal count and an
/// integer part at least as wide as the stored one, the sign is always explicit,
/// the altitude token is carried over byte for byte (it is not editable) and so
/// is the closing solidus. A field that needs more integer digits than the file
/// uses is widened — the value decides the text, and the writer prices the extra
/// bytes against the room the file itself has. The horizontal pair is not
/// validated here; the caller has already done that ([`validate_edit`]).
pub(crate) fn render_iso6709_in_shape(
    existing: &str,
    latitude: f64,
    longitude: f64,
) -> Option<Vec<u8>> {
    let shape = Iso6709Shape::parse(existing.as_bytes())?;
    let mut rendered = shape.fields[0].render(latitude);
    rendered.extend_from_slice(&shape.fields[1].render(longitude));
    if let Some(altitude) = &shape.altitude {
        rendered.extend_from_slice(altitude);
    }
    if shape.solidus {
        rendered.push(b'/');
    }
    Some(rendered)
}

/// The shape of a stored ISO 6709 position: how each horizontal field is
/// written, the altitude token verbatim, and whether a solidus closes it.
struct Iso6709Shape {
    /// Latitude and longitude, in that order.
    fields: [FieldShape; 2],
    /// The third component exactly as stored, sign and decimals included;
    /// `None` when the value has no altitude.
    altitude: Option<Vec<u8>>,
    /// Whether the value ends with `/`.
    solidus: bool,
}

/// How one field of a position is written.
struct FieldShape {
    /// Digits before the decimal separator.
    integer_digits: usize,
    /// Digits behind it, 0 when the field has none.
    decimals: usize,
}

impl Iso6709Shape {
    /// The shape of a stored value, or `None` when it is not
    /// `±digits[.digits]` twice or three times, optionally closed by `/`.
    fn parse(value: &[u8]) -> Option<Self> {
        let mut components: Vec<(FieldShape, Vec<u8>)> = Vec::new();
        let mut index = 0;
        while index < value.len() {
            if value[index] == b'/' {
                // The solidus closes the value; nothing may follow it.
                if index + 1 != value.len() || components.len() < 2 {
                    return None;
                }
                return Self::of(components, true);
            }
            let (field, end) = FieldShape::parse(value, index)?;
            components.push((field, value[index..end].to_vec()));
            index = end;
        }
        Self::of(components, false)
    }

    fn of(components: Vec<(FieldShape, Vec<u8>)>, solidus: bool) -> Option<Self> {
        let mut components = components.into_iter();
        let latitude = components.next()?.0;
        let longitude = components.next()?.0;
        let third = components.next();
        if components.next().is_some() {
            return None;
        }
        Some(Self {
            fields: [latitude, longitude],
            altitude: third.map(|(_, token)| token),
            solidus,
        })
    }
}

impl FieldShape {
    /// The field starting at `start`, and where it ends.
    fn parse(value: &[u8], start: usize) -> Option<(Self, usize)> {
        if !matches!(value.get(start), Some(b'+' | b'-')) {
            return None;
        }
        let mut index = start + 1;
        let integer_start = index;
        while value.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        let integer_digits = index - integer_start;
        if integer_digits == 0 {
            return None;
        }
        let mut decimals = 0;
        if value.get(index) == Some(&b'.') {
            index += 1;
            let fraction_start = index;
            while value.get(index).is_some_and(u8::is_ascii_digit) {
                index += 1;
            }
            decimals = index - fraction_start;
            if decimals == 0 {
                return None;
            }
        }
        Some((
            Self {
                integer_digits,
                decimals,
            },
            index,
        ))
    }

    /// `value` written in this shape: an explicit sign, the stored decimal
    /// count, and at least the stored integer width (zero-padded, never
    /// truncated).
    fn render(&self, value: f64) -> Vec<u8> {
        let sign = if value.is_sign_negative() { b'-' } else { b'+' };
        let rendered = format!("{:.*}", self.decimals, value.abs());
        let (integer, fraction) = match rendered.split_once('.') {
            Some((integer, fraction)) => (integer, Some(fraction)),
            None => (rendered.as_str(), None),
        };
        let width = integer.len().max(self.integer_digits);
        let mut out = Vec::with_capacity(2 + width + self.decimals);
        out.push(sign);
        out.resize(1 + width - integer.len(), b'0');
        out.extend_from_slice(integer.as_bytes());
        if let Some(fraction) = fraction {
            out.push(b'.');
            out.extend_from_slice(fraction.as_bytes());
        }
        out
    }
}

/// Writes `rendered` into the front of `slot` and NUL-fills the rest, so the
/// carrier's payload keeps the byte length it already has.
///
/// A rendering that does not fit is the caller's decision to make — [`NoRoom`]
/// for a container with no room to grow — so it is caught here only as a
/// violated invariant, never as a failure the caller may ignore.
///
/// [`NoRoom`]: Mp4MetadataError::NoRoom
pub(crate) fn write_payload_slot(slot: &mut [u8], rendered: &[u8]) {
    debug_assert!(
        rendered.len() <= slot.len(),
        "the rendered value must fit the payload slot it replaces"
    );
    let written = rendered.len().min(slot.len());
    slot[..written].copy_from_slice(&rendered[..written]);
    slot[written..].fill(0);
}

/// A modification time in the form the scanner stores: whole seconds, or `None`
/// when the file's own clock does not fit that form.
///
/// The refusal is the honest answer. A clock before the Unix epoch has no
/// timestamp to hand back, and the scanner reports no modification time at all
/// for such a file — so an instant invented here could never be matched against
/// the row, and every later scan would re-extract a file the save did not
/// change. [`write_metadata`] turns the `None` into a refusal before it writes.
fn truncate_to_seconds(time: SystemTime) -> Option<DateTime<Utc>> {
    let seconds = time.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    // Checked, not cast: casting turns a clock years past what `DateTime` covers
    // into a negative timestamp — a plausible-looking date rather than the
    // refusal this guard is for.
    DateTime::from_timestamp(i64::try_from(seconds).ok()?, 0)
}

/// Maps a failure to open a target for writing onto what the caller can act on.
fn open_error(err: std::io::Error) -> Mp4MetadataError {
    match err.kind() {
        ErrorKind::NotFound => Mp4MetadataError::MissingFile,
        ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem => {
            Mp4MetadataError::ReadOnly(err.to_string())
        }
        _ => Mp4MetadataError::Io(err),
    }
}

/// Names what was found where a writable container was expected.
fn extension_name(path: &Path) -> String {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) => extension.to_ascii_lowercase(),
        None => "no extension".to_string(),
    }
}

/// `original` — a `moov` region exactly as it was read out of the file — with
/// `edit` written into it, or the refusal a write would have been.
///
/// Every refusal a save can make about the region happens here, before a byte
/// is touched, and none of them looks at anything but these two arguments: no
/// clock, no randomness, no I/O. That is what lets [`restore_region`] answer
/// "does the file still hold what that save wrote?" by calling this a second
/// time — the same region and the same edit render the same bytes, whatever
/// shape the region's box header has.
fn patch_moov(original: &[u8], edit: &VideoMetadataEdit) -> Result<Vec<u8>, Mp4MetadataError> {
    let tree = parse_box_tree(original, 0, original.len(), &[])?;
    let moov = tree
        .first()
        .filter(|span| span.kind == *b"moov")
        .ok_or_else(|| Mp4MetadataError::UnsupportedContainer("moov".to_string()))?;
    if contains_kind(moov, b"mvex") {
        return Err(Mp4MetadataError::Fragmented);
    }
    let patched = rebuild_moov(original, moov, edit)?;
    if patched.len() != original.len() {
        // Only a `moov` of the same length can be written over the region it
        // came from: anything else would move the bytes behind it.
        return Err(Mp4MetadataError::NoRoom("moov"));
    }
    Ok(patched)
}

/// Renders `edit` into a `moov` of exactly the length `original` has.
fn rebuild_moov(
    original: &[u8],
    moov: &BoxSpan,
    edit: &VideoMetadataEdit,
) -> Result<Vec<u8>, Mp4MetadataError> {
    apply_edits(original, moov, carriers(original, moov, edit)?)
}

/// Every carrier this writer renders, as regions of `original`.
///
/// The fixed-width creation fields, the text carriers a save has to keep in
/// step with them, and the location carriers. All of them are rendered before
/// anything is written, so a carrier that cannot hold the new value fails the
/// write instead of leaving a half-patched file.
///
/// An instant asked for and carried by no box is refused the same way a
/// location is ([`Mp4MetadataError::NoLocationCarrier`]): a save that rendered
/// no date carrier would otherwise report an instant the file does not hold.
///
/// The `ilst` index is built once here and shared: both collectors resolve
/// their carriers against the same item list, and each building its own would
/// walk that list twice over on a save carrying both values. A save that names
/// no value resolves no carrier, so it never builds one.
fn carriers(
    buf: &[u8],
    moov: &BoxSpan,
    edit: &VideoMetadataEdit,
) -> Result<Vec<MoovEdit>, Mp4MetadataError> {
    let mut edits = Vec::new();
    if edit.taken_at.is_none() && edit.latitude.is_none() {
        return Ok(edits);
    }
    let ilst = IlstIndex::of(moov);
    if let Some(taken_at) = edit.taken_at {
        let seconds = quicktime_seconds(taken_at).ok_or(Mp4MetadataError::InvalidDate)?;
        let dated = edits.len();
        collect_time_edits(buf, moov, seconds, &mut edits, false)?;
        collect_text_date_edits(buf, moov, &ilst, taken_at, &mut edits)?;
        if edits.len() == dated {
            return Err(Mp4MetadataError::Unrepresentable(DATE_CARRIER));
        }
    }
    // `validate_edit` has already refused a position whose pair is incomplete,
    // so a latitude here always has its longitude.
    if let (Some(latitude), Some(longitude)) = (edit.latitude, edit.longitude) {
        collect_location_edits(buf, moov, &ilst, latitude, longitude, &mut edits)?;
    }
    Ok(edits)
}

/// Collects the replacement of every text date carrier at or below `moov`: the
/// mdta keys, the `©day` `ilst` item and the direct `udta/©day` child.
///
/// All of them are rendered here, before anything is written, so that a carrier
/// whose value cannot be read as a date — or that cannot hold the rendering —
/// fails the whole write. A carrier the reader skips because its payload is
/// empty is skipped here too, which is what leaves a readable file writable.
/// Nothing is left half-updated: a file whose carriers disagree about the
/// recording instant is worse than one that was refused.
fn collect_text_date_edits(
    buf: &[u8],
    moov: &BoxSpan,
    ilst: &IlstIndex<'_>,
    taken_at: DateTime<Utc>,
    edits: &mut Vec<MoovEdit>,
) -> Result<(), Mp4MetadataError> {
    // A file with no `ilst` names every key in vain: reading the name list is
    // the expensive half of resolving a carrier, and no item can answer.
    if !ilst.is_empty() {
        let keys = mdta_keys(buf, moov);
        for (position, key) in keys.iter().enumerate() {
            let Some(name) = MDTA_CREATION_DATE_KEYS
                .iter()
                .find(|name| **name == key.as_str())
            else {
                continue;
            };
            // mdta indexes are 1-based, and a key the file names twice is
            // carried by an item per position.
            let Ok(index) = u32::try_from(position + 1) else {
                continue;
            };
            for item in ilst.items(index.to_be_bytes()) {
                let Some((start, end)) = item_text_slot(buf, item) else {
                    continue;
                };
                if let Some(edit) = text_date_edit(buf, name, start, end, taken_at)? {
                    edits.push(edit);
                }
            }
        }
        for item in ilst.items(DAY_BOX) {
            let Some((start, end)) = item_text_slot(buf, item) else {
                continue;
            };
            if let Some(edit) = text_date_edit(buf, DAY_NAME, start, end, taken_at)? {
                edits.push(edit);
            }
        }
    }
    for direct in direct_udta_children(moov, &DAY_BOX) {
        if let Some(edit) = direct_text_edit(buf, DAY_NAME, direct, |text| {
            render_date_in_shape(text, taken_at)
        })? {
            edits.push(edit);
        }
    }
    Ok(())
}

/// The byte range an `ilst` item's text lives in: the payload of its `data`
/// box, behind the type-indicator/locale word — `None` for an item that has no
/// readable one.
///
/// An item whose `data` box is missing or too short to hold the word is not a
/// carrier, and skipping it is what keeps the reader and the writer in step:
/// [`ilst_items`] drops it from the values it reports, so refusing the save over
/// it would make a file the reader read without a value unwritable, and worse,
/// it would not be that file's only carrier.
fn item_text_slot(buf: &[u8], item: &BoxSpan) -> Option<(usize, usize)> {
    let data = find_child(item, b"data")?;
    let start = data.offset + header_len(buf, data.offset) + 8;
    let end = data.offset + data.size;
    (start <= end && end <= buf.len()).then_some((start, end))
}

/// Collects the replacement of every location carrier the file holds: the mdta
/// keys, the `©xyz` `ilst` item, the direct `udta/©xyz` child and the ISO/3GPP
/// `udta/loci` pair.
///
/// Every carrier is rendered — in its own shape, so they all end up naming the
/// same position — or the whole save is refused. A carrier the reader skips
/// because its payload is empty is skipped here too, so a file that reads
/// correctly can be saved to. A file with no location carrier at all cannot
/// take the position, and says so.
fn collect_location_edits(
    buf: &[u8],
    moov: &BoxSpan,
    ilst: &IlstIndex<'_>,
    latitude: f64,
    longitude: f64,
    edits: &mut Vec<MoovEdit>,
) -> Result<(), Mp4MetadataError> {
    let mut carriers = 0usize;
    // A file with no `ilst` names every key in vain: reading the name list is
    // the expensive half of resolving a carrier, and no item can answer.
    if !ilst.is_empty() {
        let keys = mdta_keys(buf, moov);
        for (position, key) in keys.iter().enumerate() {
            let Some(name) = MDTA_LOCATION_KEYS
                .iter()
                .find(|name| **name == key.as_str())
            else {
                continue;
            };
            // mdta indexes are 1-based, and a key the file names twice is
            // carried by an item per position.
            let Ok(index) = u32::try_from(position + 1) else {
                continue;
            };
            for item in ilst.items(index.to_be_bytes()) {
                let Some((start, end)) = item_text_slot(buf, item) else {
                    continue;
                };
                if let Some(edit) = location_edit(buf, name, start, end, latitude, longitude)? {
                    edits.push(edit);
                    carriers += 1;
                }
            }
        }
        for item in ilst.items(XYZ_BOX) {
            let Some((start, end)) = item_text_slot(buf, item) else {
                continue;
            };
            if let Some(edit) = location_edit(buf, XYZ_NAME, start, end, latitude, longitude)? {
                edits.push(edit);
                carriers += 1;
            }
        }
    }
    for direct in direct_udta_children(moov, &XYZ_BOX) {
        if let Some(edit) = direct_text_edit(buf, XYZ_NAME, direct, |text| {
            render_iso6709_in_shape(text, latitude, longitude)
        })? {
            edits.push(edit);
            carriers += 1;
        }
    }
    for loci in direct_udta_children(moov, b"loci") {
        if let Some(edit) = loci_edit(buf, loci, latitude, longitude)? {
            edits.push(edit);
            carriers += 1;
        }
    }
    if carriers == 0 {
        return Err(Mp4MetadataError::NoLocationCarrier);
    }
    Ok(())
}

/// The replacement of one `udta/loci` position: the horizontal pair patched in
/// place.
///
/// The pair is 8 fixed bytes — two `i32`s, no length field, no padding — so a
/// save always fits the bytes it replaces, whatever the position renders as in
/// the text carriers. `None` when the box holds no readable pair, which the
/// reader does not count as a carrier either; a refusal only when the value
/// itself cannot be encoded in the pair.
fn loci_edit(
    buf: &[u8],
    loci: &BoxSpan,
    latitude: f64,
    longitude: f64,
) -> Result<Option<MoovEdit>, Mp4MetadataError> {
    let Some(offset) = loci_pair_offset(buf, loci) else {
        return Ok(None);
    };
    let longitude =
        fixed_point_16_16(longitude).ok_or(Mp4MetadataError::Unrepresentable(LOCI_NAME))?;
    let latitude =
        fixed_point_16_16(latitude).ok_or(Mp4MetadataError::Unrepresentable(LOCI_NAME))?;
    let mut content = longitude.to_be_bytes().to_vec();
    content.extend_from_slice(&latitude.to_be_bytes());
    Ok(Some(MoovEdit {
        offset,
        replaced: 8,
        content,
        carrier: LOCI_NAME,
    }))
}

/// The replacement of one location carrier: the new position rendered in the
/// shape the carrier already holds, filling its payload slot.
///
/// An empty payload is not a carrier — the reader skips it
/// ([`first_non_empty`]) — so it is answered as nothing to replace rather than
/// as a value the format cannot express: a file that reads correctly has to
/// stay writable.
fn location_edit(
    buf: &[u8],
    carrier: &'static str,
    start: usize,
    end: usize,
    latitude: f64,
    longitude: f64,
) -> Result<Option<MoovEdit>, Mp4MetadataError> {
    let slot = buf
        .get(start..end)
        .ok_or(Mp4MetadataError::Unrepresentable(carrier))?;
    let existing = text_of(slot);
    if existing.is_empty() {
        return Ok(None);
    }
    let rendered = render_iso6709_in_shape(&existing, latitude, longitude)
        .ok_or(Mp4MetadataError::Unrepresentable(carrier))?;
    Ok(Some(slot_edit(carrier, start, slot.len(), rendered)))
}

/// A carrier's replacement: `rendered` NUL-padded to the payload slot it fills,
/// or left as it is when the rendering needs more bytes than the slot has — the
/// rebuild is what prices that growth.
fn slot_edit(carrier: &'static str, offset: usize, replaced: usize, rendered: Vec<u8>) -> MoovEdit {
    let mut content = vec![0u8; replaced.max(rendered.len())];
    write_payload_slot(&mut content, &rendered);
    MoovEdit {
        offset,
        replaced,
        content,
        carrier,
    }
}

/// The replacement of one text date carrier: the new instant rendered in the
/// shape the carrier already holds, filling its payload slot.
///
/// An empty payload is not a carrier — the reader skips it
/// ([`first_non_empty`]) — so it is answered as nothing to replace rather than
/// as a value the format cannot express.
fn text_date_edit(
    buf: &[u8],
    carrier: &'static str,
    start: usize,
    end: usize,
    taken_at: DateTime<Utc>,
) -> Result<Option<MoovEdit>, Mp4MetadataError> {
    let slot = buf
        .get(start..end)
        .ok_or(Mp4MetadataError::Unrepresentable(carrier))?;
    let existing = text_of(slot);
    if existing.is_empty() {
        return Ok(None);
    }
    let rendered = render_date_in_shape(&existing, taken_at)
        .ok_or(Mp4MetadataError::Unrepresentable(carrier))?;
    Ok(Some(slot_edit(carrier, start, slot.len(), rendered)))
}

/// The replacement of one direct `udta` text child, rendered in the shape the
/// child already holds.
///
/// The whole payload is replaced, so a text atom's byte count and language code
/// are written with the text: a rendering that needs more bytes than the text
/// had leaves the count telling the truth about what follows it, and one of the
/// same length leaves the payload byte-identical to the one that was there. The
/// count covers the payload the slot is padded to, not the text alone — the
/// reader recognises a text atom by that word alone
/// ([`direct_text_slot`]), so a count short of the NUL padding would demote the
/// atom to raw text and strand every save after this one. A raw-text child has
/// no header, so its payload is the rendering alone — as it was before text
/// atoms were recognized. An empty payload is not a carrier, so it is answered
/// as nothing to replace.
fn direct_text_edit(
    buf: &[u8],
    carrier: &'static str,
    direct: &BoxSpan,
    render: impl FnOnce(&str) -> Option<Vec<u8>>,
) -> Result<Option<MoovEdit>, Mp4MetadataError> {
    let (payload, text) = direct_text_slot(buf, direct);
    // A text atom keeps its byte count and language code in front of the text.
    let text_atom = payload.start < text.start;
    let slot = buf
        .get(text)
        .ok_or(Mp4MetadataError::Unrepresentable(carrier))?;
    let existing = text_of(slot);
    if existing.is_empty() {
        return Ok(None);
    }
    let rendered = render(&existing).ok_or(Mp4MetadataError::Unrepresentable(carrier))?;
    let replaced = payload.end - payload.start;
    let content = if text_atom {
        let length = u16::try_from(replaced.max(4 + rendered.len()) - 4)
            .map_err(|_| Mp4MetadataError::Unrepresentable(carrier))?;
        let language = buf
            .get(payload.start + 2..payload.start + 4)
            .ok_or(Mp4MetadataError::Unrepresentable(carrier))?;
        let mut content = Vec::with_capacity(4 + rendered.len());
        content.extend_from_slice(&length.to_be_bytes());
        content.extend_from_slice(language);
        content.extend_from_slice(&rendered);
        content
    } else {
        rendered
    };
    Ok(Some(slot_edit(carrier, payload.start, replaced, content)))
}

/// Collects the replacement of every `mvhd`/`tkhd`/`mdhd` creation field at or
/// below `span`, and of none inside an `ilst`.
///
/// A creation field is a carrier on the path a reader resolves it on: a
/// `moov`/`mvhd`, a `trak`/`tkhd`, a `mdia`/`mdhd`. An `ilst` child is not on
/// one — it is a metadata item whose type is a four-character code, and any
/// file may spend `mvhd` on one of its own. `is_container` parses every such
/// item as a box, so a writer matching the name alone wrote the new instant
/// into the item's body+4 — the type field of the `data` box the item is made
/// of — rewriting bytes no reader resolves and leaving the item unreadable
/// behind.
fn collect_time_edits(
    buf: &[u8],
    span: &BoxSpan,
    seconds: u32,
    edits: &mut Vec<MoovEdit>,
    in_item_list: bool,
) -> Result<(), Mp4MetadataError> {
    if !in_item_list && TIME_BOXES.contains(&span.kind) {
        edits.push(time_edit(buf, span, seconds)?);
    }
    for child in &span.children {
        collect_time_edits(
            buf,
            child,
            seconds,
            edits,
            in_item_list || span.kind == *b"ilst",
        )?;
    }
    Ok(())
}

/// The replacement of one creation field: the same reach as the field, filled
/// with `seconds` in the width the box version declares.
fn time_edit(buf: &[u8], span: &BoxSpan, seconds: u32) -> Result<MoovEdit, Mp4MetadataError> {
    let body = span.offset + header_len(buf, span.offset);
    let end = span.offset + span.size;
    // version byte, then the creation field: 32 bits for version 0, 64 for 1.
    let version = *buf.get(body).ok_or(Mp4MetadataError::InvalidDate)?;
    let width = if version == 1 { 8 } else { 4 };
    if body + 4 + width > end {
        return Err(Mp4MetadataError::InvalidDate);
    }
    let content = if width == 8 {
        u64::from(seconds).to_be_bytes().to_vec()
    } else {
        seconds.to_be_bytes().to_vec()
    };
    Ok(MoovEdit {
        offset: body + 4,
        replaced: width,
        content,
        carrier: time_box_name(&span.kind),
    })
}

/// The name of one of [`TIME_BOXES`], as a failure names the carrier it could
/// not fill. Only `mvhd`, `tkhd` and `mdhd` are ever passed.
fn time_box_name(kind: &[u8; 4]) -> &'static str {
    if *kind == *b"mvhd" {
        "mvhd"
    } else if *kind == *b"tkhd" {
        "tkhd"
    } else {
        "mdhd"
    }
}

/// Applies `edits` to `original`.
///
/// A save whose renderings all fit the slots they replace is spliced in place
/// and the `moov` keeps its layout byte for byte. One whose renderings need
/// more bytes than their slots hold is paid for out of the file's own padding
/// ([`rebuild_with_padding`]). The `moov` keeps its length either way, so no
/// byte outside it moves and the media is never shifted.
fn apply_edits(
    original: &[u8],
    moov: &BoxSpan,
    mut edits: Vec<MoovEdit>,
) -> Result<Vec<u8>, Mp4MetadataError> {
    edits.sort_by_key(|edit| edit.offset);
    let mut cursor = 0usize;
    for edit in &edits {
        // Overlapping regions, or one that runs off the buffer, cannot be
        // rendered meaningfully: refuse rather than splice something wrong.
        if edit.offset < cursor || edit.offset + edit.replaced > original.len() {
            return Err(Mp4MetadataError::NoRoom(edit.carrier));
        }
        cursor = edit.offset + edit.replaced;
    }
    let growth: isize = edits.iter().map(MoovEdit::delta).sum();
    if growth == 0 {
        return Ok(splice(original, &edits));
    }
    rebuild_with_padding(original, moov, &edits, growth)
}

/// `original` with every edit written at the offset it was measured at.
fn splice(original: &[u8], edits: &[MoovEdit]) -> Vec<u8> {
    let mut patched = Vec::with_capacity(original.len());
    let mut cursor = 0usize;
    for edit in edits {
        patched.extend_from_slice(&original[cursor..edit.offset]);
        patched.extend_from_slice(&edit.content);
        cursor = edit.offset + edit.replaced;
    }
    patched.extend_from_slice(&original[cursor..]);
    debug_assert_eq!(patched.len(), original.len());
    patched
}

/// Whether `kind` is a box a file keeps only as padding.
fn is_padding(kind: &[u8; 4]) -> bool {
    *kind == *b"free" || *kind == *b"skip"
}

/// What the file's own padding offers a rendering that did not fit its slot.
#[derive(Debug, Clone, Copy, Default)]
struct Padding {
    /// Bytes the `free`/`skip` boxes occupy, headers included: what a rebuild
    /// has to cover to leave the `moov` its length.
    total: usize,
}

impl Padding {
    /// The padding at or below `span`.
    fn of(span: &BoxSpan) -> Self {
        let mut padding = Self::default();
        padding.collect(span);
        padding
    }

    fn collect(&mut self, span: &BoxSpan) {
        if is_padding(&span.kind) {
            self.total += span.size;
        }
        for child in &span.children {
            self.collect(child);
        }
    }
}

/// Rebuilds `moov` with the renderings that did not fit their slots, paid for
/// out of the file's own padding.
///
/// Every `free`/`skip` box inside the `moov` is dropped from its children and
/// their bytes are re-emitted as one `free` box behind them — the `moov`'s last
/// child, in front of any trailing bytes the walk behind the last child does not
/// read — so the `moov` keeps its exact length and nothing outside it ever
/// moves. What a save may spend is the whole of that padding — the bytes behind
/// the boxes' headers come back with them — but what is emitted has to be a box:
/// a growth that needs more than the file has is refused, and so is one that
/// would leave 1..=7 bytes behind, because a `free` box is 8 bytes at its
/// smallest. The edits keep the offsets they were measured at, because they are
/// applied while the tree is walked rather than against a buffer: an earlier
/// carrier's growth cannot displace a later one.
fn rebuild_with_padding(
    original: &[u8],
    moov: &BoxSpan,
    edits: &[MoovEdit],
    growth: isize,
) -> Result<Vec<u8>, Mp4MetadataError> {
    let padding = Padding::of(moov);
    let filler = padding.total as isize - growth;
    if filler < 0 || (1..=7).contains(&filler) {
        return Err(Mp4MetadataError::NoRoom(growing_carrier(edits)));
    }
    let header = header_len(original, moov.offset);
    let mut out = Vec::with_capacity(original.len());
    // The `moov` header is kept verbatim: the rebuild adds up to exactly the
    // length the box already declares.
    out.extend_from_slice(&original[moov.offset..moov.offset + header]);
    if filler > 0 {
        let filler = usize::try_from(filler).map_err(|_| Mp4MetadataError::NoRoom("moov"))?;
        // The filler is a box *child* of the `moov`, so it goes in front of the
        // bytes behind the last child: a container may legally end with 1..=7
        // unparsed trailing bytes, and a filler emitted after them would not be
        // the last child the walk that reads the rebuild back finds.
        let tail = emit_children(original, moov, edits, &mut out);
        out.extend_from_slice(&free_box(filler));
        out.extend_from_slice(&original[tail..moov.offset + moov.size]);
    } else {
        emit_body(original, moov, edits, &mut out);
    }
    if out.len() != original.len() {
        // Length preservation is the whole safety argument of this path; a
        // rebuild that missed it must not be written.
        return Err(Mp4MetadataError::NoRoom("moov"));
    }
    Ok(out)
}

/// The carrier a growth refusal names: the first one that needed more bytes
/// than its slot held.
fn growing_carrier(edits: &[MoovEdit]) -> &'static str {
    edits
        .iter()
        .find(|edit| edit.delta() > 0)
        .map_or("moov", |edit| edit.carrier)
}

/// A `free` box of exactly `size` bytes with a zeroed payload.
fn free_box(size: usize) -> Vec<u8> {
    let size = size.max(8);
    let mut out = vec![0u8; size];
    out[..4].copy_from_slice(&(size as u32).to_be_bytes());
    out[4..8].copy_from_slice(b"free");
    out
}

/// Emits `span`'s payload — the gaps between its children, the children
/// themselves and the tail — applying `edits` and dropping padding boxes.
fn emit_body(buf: &[u8], span: &BoxSpan, edits: &[MoovEdit], out: &mut Vec<u8>) {
    let start = span.offset + header_len(buf, span.offset);
    let end = span.offset + span.size;
    if span.children.is_empty() {
        let mut cursor = start;
        for edit in edits
            .iter()
            .filter(|edit| edit.offset >= start && edit.offset + edit.replaced <= end)
        {
            out.extend_from_slice(&buf[cursor..edit.offset]);
            out.extend_from_slice(&edit.content);
            cursor = edit.offset + edit.replaced;
        }
        out.extend_from_slice(&buf[cursor..end]);
        return;
    }
    let tail = emit_children(buf, span, edits, out);
    out.extend_from_slice(&buf[tail..end]);
}

/// Emits `span`'s children, the bytes between and around them included, and
/// answers where the bytes behind the last child start.
///
/// Advancing past a dropped padding box is what keeps its bytes out of the
/// rebuild — they are re-emitted as one `free` box. A container may legally end
/// with 1..=7 bytes that are not a child; they are the caller's to place
/// (`emit_body` copies them behind everything, a rebuild with a filler needs
/// them behind the filler).
fn emit_children(buf: &[u8], span: &BoxSpan, edits: &[MoovEdit], out: &mut Vec<u8>) -> usize {
    let start = span.offset + header_len(buf, span.offset);
    let mut cursor = start;
    for child in &span.children {
        // A `meta` body opens with a version/flags word when the file writes
        // the ISO-BMFF form: the bytes in front of each child are copied, not
        // assumed away.
        out.extend_from_slice(&buf[cursor..child.offset]);
        if !is_padding(&child.kind) {
            emit_box(buf, child, edits, out);
        }
        cursor = child.offset + child.size;
    }
    cursor
}

/// Emits `span` as a box whose size field says how long it now is.
fn emit_box(buf: &[u8], span: &BoxSpan, edits: &[MoovEdit], out: &mut Vec<u8>) {
    let header = header_len(buf, span.offset);
    let size = span.size as isize + span_delta(span, edits);
    debug_assert!(size >= header as isize && size <= u32::MAX as isize);
    let size = size as usize;
    if header == 16 {
        // The 64-bit form keeps its header shape: `1`, the type, then `largesize`.
        out.extend_from_slice(&1u32.to_be_bytes());
        out.extend_from_slice(&span.kind);
        out.extend_from_slice(&(size as u64).to_be_bytes());
    } else {
        out.extend_from_slice(&(size as u32).to_be_bytes());
        out.extend_from_slice(&span.kind);
    }
    emit_body(buf, span, edits, out);
}

/// The bytes `span` gains (or loses) in a rebuild: its edits' deltas, less the
/// padding boxes dropped from inside it.
fn span_delta(span: &BoxSpan, edits: &[MoovEdit]) -> isize {
    let edited: isize = edits
        .iter()
        .filter(|edit| {
            edit.offset >= span.offset && edit.offset + edit.replaced <= span.offset + span.size
        })
        .map(MoovEdit::delta)
        .sum();
    edited - Padding::of(span).total as isize
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn reads_apple_key_carriers_from_the_fixture() {
        let m = read_metadata(Path::new("test-data/test_video_quicktime_keys.mp4")).unwrap();
        assert_eq!(m.location_iso6709.as_deref(), Some("+48.2082+016.3737/"));
        assert_eq!(
            m.creation_date_text.as_deref(),
            Some("2024-05-01T10:00:00+0200")
        );
        assert_eq!(m.creation_time, None); // mvhd creation_time is 0 in this fixture
    }

    #[test]
    fn reads_mvhd_creation_time_from_the_date_fixture() {
        let m = read_metadata(Path::new("test-data/test_video_with_date.mp4")).unwrap();
        assert_eq!(
            m.creation_time.unwrap().to_rfc3339(),
            "2023-06-15T10:00:00+00:00"
        );
        assert_eq!(m.creation_date_text, None);
        assert_eq!(m.location_iso6709, None);
    }

    #[test]
    fn an_undecodable_movie_header_costs_only_the_instant_it_carries() {
        // A `moov` whose creation field does not fit the instant it is decoded
        // in must not take the rest of the read down with it: the text carriers
        // the same read resolved are still there, and the scanner falls back to
        // them exactly as it does for a file with no `mvhd` at all. The instant
        // is absent, not wrong.
        let ilst = box_bytes(
            b"ilst",
            &[
                text_item_box(&XYZ_BOX, b"+48.2082+016.3737/"),
                text_item_box(&DAY_BOX, b"2024-05-01T10:00:00+0200"),
            ]
            .concat(),
        );
        let mut meta_body = vec![0u8, 0, 0, 0]; // version + flags
        meta_body.extend_from_slice(&ilst);
        let (_dir, path) = synthetic_version_1_time_file(
            &box_bytes(b"udta", &box_bytes(b"meta", &meta_body)),
            u64::MAX, // wider than the `i64` the instant is decoded in
            u64::MAX,
        );

        let read = read_metadata(&path).unwrap();
        assert_eq!(read.creation_time, None, "the instant is not guessed");
        assert_eq!(
            read.creation_date_text.as_deref(),
            Some("2024-05-01T10:00:00+0200")
        );
        assert_eq!(read.location_iso6709.as_deref(), Some("+48.2082+016.3737/"));
    }

    #[test]
    fn reads_a_moov_that_follows_the_media_data() {
        // The same recording with `moov` at the end: the scan has to walk past
        // a 1.7 MB `mdat` header-wise and still find the metadata.
        let m = read_metadata(Path::new("test-data/test_video_moov_end.mp4")).unwrap();
        assert_eq!(m.creation_time, None); // mvhd creation_time is 0
        assert_eq!(m.creation_date_text, None); // only a ©too item
        assert_eq!(m.location_iso6709, None);
    }

    #[test]
    fn refuses_non_iso_bmff_bytes_and_unlisted_extensions() {
        assert!(matches!(
            read_metadata(Path::new("test-data/test_video_long.mkv")),
            Err(Mp4MetadataError::UnsupportedContainer(_))
        ));
        assert!(matches!(
            read_metadata(Path::new("test-data/test_video_legacy.avi")),
            Err(Mp4MetadataError::UnsupportedContainer(_))
        ));
        assert!(!has_writable_extension(Path::new("/x/a.3gp")));
        assert!(has_writable_extension(Path::new("/x/a.MOV")));
    }

    #[test]
    fn parses_iso6709_forms_and_rejects_garbage() {
        assert_eq!(
            parse_iso6709("+48.2082+016.3737/"),
            Some((48.2082, 16.3737))
        );
        assert_eq!(
            parse_iso6709("-33.8688+151.2093/"),
            Some((-33.8688, 151.2093))
        );
        assert_eq!(
            parse_iso6709("+48.2082+016.3737+150.00/").unwrap().1,
            16.3737
        );
        assert_eq!(parse_iso6709("nonsense"), None);
        assert_eq!(parse_iso6709("+91.0+016.0/"), None); // out of range
    }

    #[test]
    fn strips_trailing_nul_padding_from_box_text() {
        assert_eq!(text_of(b"encoder"), "encoder");
        assert_eq!(text_of(b"encoder\0"), "encoder");
        // All of it, not just the last byte: a shorter rendering leaves its
        // whole remainder as padding, and the value has to read back clean.
        assert_eq!(text_of(b"encoder\0\0\0"), "encoder");
        assert_eq!(text_of(b"a b "), "a b "); // never trims other whitespace
    }

    #[test]
    fn parses_a_quicktime_meta_without_version_flags() {
        // QuickTime writes `udta/meta` without the ISO version/flags word: the
        // body starts directly with the `hdlr` child header. The fallback must
        // not swallow those 4 bytes as version/flags.
        let bare = qt_meta_body_without_version_flags();
        let mut quicktime = Vec::new();
        quicktime.extend_from_slice(&box_bytes(b"meta", &bare));

        let tree = parse_box_tree(&quicktime, 0, quicktime.len(), &[]).unwrap();
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].kind, *b"meta");
        assert_eq!(
            tree[0].children.iter().map(|c| c.kind).collect::<Vec<_>>(),
            vec![*b"hdlr", *b"keys", *b"ilst"]
        );

        // The ISO FullBox form: the same children behind a version/flags word.
        let mut full_box = vec![0u8, 0, 0, 0];
        full_box.extend_from_slice(&bare);
        let mut iso = Vec::new();
        iso.extend_from_slice(&box_bytes(b"meta", &full_box));
        let tree = parse_box_tree(&iso, 0, iso.len(), &[]).unwrap();
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].kind, *b"meta");
        assert_eq!(
            tree[0].children.iter().map(|c| c.kind).collect::<Vec<_>>(),
            vec![*b"hdlr", *b"keys", *b"ilst"]
        );
    }

    #[test]
    fn refuses_a_box_past_eof_and_an_empty_file() {
        let dir = TempDir::new().unwrap();

        let empty = dir.path().join("empty.mp4");
        fs::write(&empty, []).unwrap();
        assert!(matches!(
            read_metadata(&empty),
            Err(Mp4MetadataError::UnsupportedContainer(_))
        ));

        // A first box claiming far more bytes than the file holds.
        let inenar = dir.path().join("inenar.mp4");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x00FF_FFFFu32.to_be_bytes());
        bytes.extend_from_slice(b"ftyp");
        bytes.extend_from_slice(&[0u8; 16]);
        fs::write(&inenar, &bytes).unwrap();
        assert!(matches!(
            read_metadata(&inenar),
            Err(Mp4MetadataError::UnsupportedContainer(_))
        ));
    }

    #[test]
    fn reads_a_moov_at_the_end_behind_a_largesize_box() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("synthetic.mp4");
        let mut bytes = ftyp_box();
        // A `free` box in the 64-bit `largesize` form: the size field is 1 and
        // the real size follows the type.
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.extend_from_slice(b"free");
        bytes.extend_from_slice(&24u64.to_be_bytes());
        bytes.extend_from_slice(&[0u8; 8]);
        // `moov` with a size-0 header: it runs to EOF.
        let mut moov = vec![0u8, 0, 0, 0];
        moov.extend_from_slice(b"moov");
        moov.extend_from_slice(&box_bytes(b"mvhd", &mvhd_body(3_769_668_000)));
        bytes.extend_from_slice(&moov);
        fs::write(&path, &bytes).unwrap();

        let m = read_metadata(&path).unwrap();
        assert_eq!(
            m.creation_time.unwrap().to_rfc3339(),
            "2023-06-15T10:00:00+00:00"
        );
    }

    #[test]
    fn reads_a_moov_behind_padding_in_front_of_the_brand() {
        // Padding in front of the `ftyp` leaves a perfectly readable container:
        // `free`/`skip` are the boxes a writer uses as spacers and QuickTime's
        // `wide` is one of the same kind. The sniff that tells an ISO-BMFF file
        // from a foreign container is aimed at the box that carries the brand,
        // so it has to step over a box that says nothing about the container
        // rather than refuse the file on it. A first box that is neither
        // padding nor one this family opens with is still refused — that is the
        // sniff doing its job.
        let (_dir, path) = temp_copy(
            "padded_brand.mp4",
            "test-data/test_video_quicktime_keys.mp4",
        );
        let bytes = fs::read(&path).unwrap();
        for (kind, payload) in [
            (b"free", vec![0u8; 16]),
            (b"skip", vec![0u8; 16]),
            (b"wide", Vec::new()),
        ] {
            let mut padded = box_bytes(kind, &payload);
            padded.extend_from_slice(&bytes);
            fs::write(&path, &padded).unwrap();
            let read = read_metadata(&path)
                .unwrap_or_else(|err| panic!("{}: {err}", String::from_utf8_lossy(kind)));
            assert_eq!(
                read.location_iso6709.as_deref(),
                Some("+48.2082+016.3737/"),
                "{} ahead of the brand hides nothing",
                String::from_utf8_lossy(kind)
            );
        }

        let mut foreign = box_bytes(b"junk", &[0u8; 16]);
        foreign.extend_from_slice(&bytes);
        fs::write(&path, &foreign).unwrap();
        assert!(
            matches!(
                read_metadata(&path),
                Err(Mp4MetadataError::UnsupportedContainer(_))
            ),
            "a first box that is not padding is still the end of the sniff"
        );
    }

    #[test]
    fn a_brandless_quicktime_file_reads_and_takes_a_save() {
        // `ftyp` is where the ISO-BMFF family names its brand, and a classic
        // QuickTime `.mov` has none: `wide`, `mdat` and `moov` is the whole
        // spelling, and the movie header may lead. Refusing such a file left
        // `mov` in `WRITABLE_EXTENSIONS` and `has_writable_extension` answering
        // true for a file neither this reader nor this writer could open — and
        // QuickTime is a defined brand of this very family, so the refusal was
        // stricter than the sniff's purpose. Both lead shapes have to work, on
        // read and on write, or the date a save writes cannot be read back.
        let udta = box_bytes(b"udta", &box_bytes(&DAY_BOX, b"2024-05-01 10:00:00"));
        let moov_body = skeleton_moov_body(&udta, 0);
        let media_first = [box_bytes(b"wide", &[]), box_bytes(b"mdat", &[0u8; 64])].concat();
        for (name, prefix) in [
            ("moov_first.mov", Vec::new()),
            ("media_first.mov", media_first),
        ] {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join(name);
            let mut bytes = prefix;
            bytes.extend_from_slice(&MoovSizeForm::Wide32.header(moov_body.len()));
            bytes.extend_from_slice(&moov_body);
            fs::write(&path, &bytes).unwrap();

            assert_eq!(
                read_metadata(&path)
                    .unwrap_or_else(|err| panic!("{name}: {err}"))
                    .creation_date_text
                    .as_deref(),
                Some("2024-05-01 10:00:00"),
                "{name}: a file that names no brand is still this container"
            );
            write_metadata(&path, &date_edit()).unwrap();
            assert_eq!(
                read_metadata(&path).unwrap().creation_date_text.as_deref(),
                Some("2024-07-04 12:00:00"),
                "{name}: and a save lands in it"
            );
        }
    }

    #[test]
    fn refuses_a_fragmented_moov() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fragmented.mp4");
        let mut moov = box_bytes(b"mvhd", &mvhd_body(0));
        moov.extend_from_slice(&box_bytes(b"mvex", &[]));
        let mut bytes = ftyp_box();
        bytes.extend_from_slice(&box_bytes(b"moov", &moov));
        fs::write(&path, &bytes).unwrap();
        assert!(matches!(
            read_metadata(&path),
            Err(Mp4MetadataError::Fragmented)
        ));
    }

    #[test]
    fn refuses_a_moov_larger_than_the_read_limit() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("huge_moov.mp4");
        let ftyp = ftyp_box();
        let moov_size = 70 * 1024 * 1024u64; // over the 64 MiB cap
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(&ftyp).unwrap();
        file.write_all(&u32::try_from(moov_size).unwrap().to_be_bytes())
            .unwrap();
        file.write_all(b"moov").unwrap();
        // Sparse: the size has to be real for the box to be in range, but no
        // page of it is ever read.
        file.set_len(ftyp.len() as u64 + moov_size).unwrap();
        drop(file);

        assert!(matches!(
            read_metadata(&path),
            Err(Mp4MetadataError::NoRoom("moov"))
        ));
    }

    #[test]
    fn refuses_a_largesize_that_would_wrap_the_file_scan() {
        // Two 64-bit-size boxes where the second declares `2^64 - 40` bytes:
        // `offset + size` wraps to 0, so an unchecked bound accepts it and the
        // scan cycles between the two boxes forever (or dies on the overflow in
        // a debug build). The declared size must be judged against the bytes
        // left in the file instead.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("wrapping.mp4");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.extend_from_slice(b"ftyp");
        bytes.extend_from_slice(&40u64.to_be_bytes());
        bytes.extend_from_slice(&[0u8; 24]);
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.extend_from_slice(b"free");
        bytes.extend_from_slice(&(u64::MAX - 39).to_be_bytes());
        bytes.extend_from_slice(&[0u8; 8]);
        assert_eq!(bytes.len(), 64);
        fs::write(&path, &bytes).unwrap();

        assert_terminates_within(move || {
            matches!(
                read_metadata(&path),
                Err(Mp4MetadataError::UnsupportedContainer(_))
            )
        });
    }

    #[test]
    fn refuses_a_box_size_that_would_wrap_the_region() {
        // An 8-byte box followed by one whose 64-bit size is `2^64 - 8`: added
        // to its own offset it wraps to 0, which an unchecked region bound would
        // accept and then re-walk the same bytes forever.
        let mut buf = box_bytes(b"free", &[]);
        buf.extend_from_slice(&1u32.to_be_bytes());
        buf.extend_from_slice(b"trak");
        buf.extend_from_slice(&(u64::MAX - 7).to_be_bytes());
        buf.extend_from_slice(&[0u8; 8]);
        let len = buf.len();

        assert_terminates_within(move || {
            matches!(
                parse_box_tree(&buf, 0, len, &[]),
                Err(Mp4MetadataError::UnsupportedContainer(_))
            )
        });
    }

    #[test]
    fn refuses_a_nesting_chain_deeper_than_the_cap() {
        // A crafted file can nest containers until the walker's recursion dies.
        // The cap is checked in front of the recursive call, so a chain of
        // `MAX_BOX_DEPTH` containers is walked and the next container deeper is
        // refused rather than followed. Both sides of that boundary are pinned
        // here, in two nesting contexts: showing only the refusal would still
        // pass with the cap set anywhere below the depth the case happened to
        // use.
        let nested = |depth: usize| {
            let mut body = Vec::new();
            for _ in 0..depth {
                body = box_bytes(b"trak", &body);
            }
            body
        };
        let at_the_cap = nested(MAX_BOX_DEPTH);
        assert!(
            parse_box_tree(&at_the_cap, 0, at_the_cap.len(), &[]).is_ok(),
            "a chain as deep as the cap is still walked"
        );
        let past_the_cap = nested(MAX_BOX_DEPTH + 1);
        assert!(
            matches!(
                parse_box_tree(&past_the_cap, 0, past_the_cap.len(), &[]),
                Err(Mp4MetadataError::UnsupportedContainer(found)) if found == "box nesting depth"
            ),
            "one container past the cap is refused, not walked"
        );

        // The same boundary inside a real file, where the enclosing `moov` is
        // itself one level of the cap: the deepest chain the read still walks is
        // one container shorter, and the one after that is the refusal.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nested.mp4");
        let mut walked = ftyp_box();
        walked.extend_from_slice(&box_bytes(b"moov", &nested(MAX_BOX_DEPTH - 1)));
        fs::write(&path, &walked).unwrap();
        assert!(
            read_metadata(&path).is_ok(),
            "a chain that fits the cap is read, `moov` level included"
        );
        let mut refused = ftyp_box();
        refused.extend_from_slice(&box_bytes(b"moov", &nested(MAX_BOX_DEPTH)));
        fs::write(&path, &refused).unwrap();
        let err = read_metadata(&path).unwrap_err();
        assert!(
            matches!(&err, Mp4MetadataError::UnsupportedContainer(found) if found == "box nesting depth"),
            "{err}"
        );
    }

    #[test]
    fn refuses_a_container_with_more_boxes_than_the_cap() {
        // The depth cap bounds how deep a crafted file can nest; before the count
        // cap nothing bounded how many boxes it could hold. A `moov` is capped at
        // 64 MiB, and a region filled with nothing but minimal headers holds
        // eight million of them — each one a span, with its own children vector,
        // in memory: several times what it costs in the buffer it came from. The
        // budget is spent across every level, so nesting cannot buy more than a
        // flat walk gets.
        let packed = |boxes: usize| {
            let mut buf = Vec::with_capacity(boxes * 8);
            (0..boxes).for_each(|_| buf.extend_from_slice(&box_bytes(b"free", &[])));
            buf
        };
        let at_the_cap = packed(MAX_BOXES);
        assert_eq!(
            parse_box_tree(&at_the_cap, 0, at_the_cap.len(), &[])
                .map(|tree| tree.len())
                .unwrap_or(0),
            MAX_BOXES,
            "a walk as large as the cap is still walked"
        );

        let over_the_cap = packed(MAX_BOXES + 1);
        assert!(
            matches!(
                parse_box_tree(&over_the_cap, 0, over_the_cap.len(), &[]),
                Err(Mp4MetadataError::UnsupportedContainer(found)) if found == "box count"
            ),
            "one box past the cap is refused rather than walked"
        );

        // The same file, reached through the read: the caller sees the refusal
        // instead of a tree built out of the whole region.
        let (_dir, path) = synthetic_file_with_moov(&over_the_cap, MoovSizeForm::Wide32);
        let err = read_metadata(&path).unwrap_err();
        assert!(
            matches!(&err, Mp4MetadataError::UnsupportedContainer(found) if found == "box count"),
            "{err}"
        );
    }

    #[test]
    fn a_keys_box_names_only_what_it_can_hold() {
        // The entry count of `moov/udta/meta/keys` is a field in the file, and an
        // entry costs as little as eight bytes, so a count walked as it reads is
        // a `String` per eight bytes of `moov` — on every read of every video on
        // every scan, on top of what the 64 MiB region already costs. The walk
        // is bounded by what the box holds and by the number of names an `ilst`
        // could index at all.
        let keys_declaring = |entries: usize| {
            let mut body = vec![0u8, 0, 0, 0]; // version + flags
            body.extend_from_slice(&u32::MAX.to_be_bytes()); // entry count
            for _ in 0..entries {
                // The smallest entry there is: a size covering itself and the
                // namespace, and no key bytes behind it.
                body.extend_from_slice(&box_bytes(b"mdta", &[]));
            }
            let mut meta = vec![0u8, 0, 0, 0]; // version + flags
            meta.extend_from_slice(&box_bytes(b"keys", &body));
            box_bytes(b"moov", &box_bytes(b"udta", &box_bytes(b"meta", &meta)))
        };
        let names = |entries: usize| {
            let bytes = keys_declaring(entries);
            let tree = parse_box_tree(&bytes, 0, bytes.len(), &[]).unwrap();
            mdta_keys(&bytes, tree.first().unwrap())
        };

        // GIVEN a box that declares `u32::MAX` entries over three of them
        assert_eq!(
            names(3).len(),
            3,
            "only the entries the box physically holds are walked"
        );

        // AND one packed past what a parsed item list could ever index
        assert_eq!(
            names(MAX_MDTA_KEYS + 32).len(),
            MAX_MDTA_KEYS,
            "the walk stops at the name budget, not at the declared count"
        );

        // The read that pays for it: a file carrying that box still answers, and
        // the eight-byte entries it is packed with are not a date.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("packed_keys.mp4");
        let mut file = ftyp_box();
        file.extend_from_slice(&keys_declaring(MAX_MDTA_KEYS + 32));
        fs::write(&path, &file).unwrap();
        assert_terminates_within(move || {
            read_metadata(&path).is_ok_and(|read| read.creation_date_text.is_none())
        });
    }

    #[test]
    fn refuses_a_file_of_nothing_but_placeholder_boxes() {
        // `locate_moov` reads one header per top-level box, and `is_placeholder`
        // exempts `free`/`skip`/`wide` from the sniff — so a file built
        // entirely out of them advances eight bytes per seek and read, and
        // without a budget the scan pays a syscall pair per eight bytes of the
        // file before it can refuse. A real file never looks like this; a
        // crafted one can be gigabytes of it.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("padding.mp4");
        let mut bytes = Vec::with_capacity((MAX_BOXES + 1) * 8);
        for _ in 0..=MAX_BOXES {
            bytes.extend_from_slice(&box_bytes(b"free", &[]));
        }
        fs::write(&path, &bytes).unwrap();
        assert_terminates_within(move || {
            matches!(
                read_metadata(&path).unwrap_err(),
                Mp4MetadataError::UnsupportedContainer(found) if found == "box count"
            )
        });
    }

    #[test]
    fn patching_the_date_writes_only_the_moov_region() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("clip.mp4");
        fs::copy("test-data/test_video_with_date.mp4", &path).unwrap();
        let before = fs::read(&path).unwrap();
        let before_mtime = fs::metadata(&path).unwrap().modified().unwrap();

        let edit = VideoMetadataEdit {
            taken_at: Some("2024-07-04T12:00:00Z".parse().unwrap()),
            ..Default::default()
        };
        let out = write_metadata(&path, &edit).unwrap();
        let after = fs::read(&path).unwrap();

        assert_eq!(after.len(), before.len(), "the file length must not change");
        let (moov_off, moov_len) = locate_moov_in(&before).unwrap();
        assert_eq!(&after[..moov_off], &before[..moov_off]);
        assert_eq!(
            &after[moov_off + moov_len..],
            &before[moov_off + moov_len..]
        );
        assert_ne!(
            &after[moov_off..moov_off + moov_len],
            &before[moov_off..moov_off + moov_len],
            "the patched moov must differ"
        );
        // Not just "outside moov": every changed byte is inside a creation field.
        let fields: Vec<_> = date_fields(&before)
            .into_iter()
            .map(|(_, offset, width)| offset..offset + width)
            .collect();
        for offset in differing_offsets(&before, &after) {
            assert!(
                fields.iter().any(|field| field.contains(&offset)),
                "byte {offset} outside every creation field changed"
            );
        }

        assert_eq!(
            fs::metadata(&path).unwrap().modified().unwrap(),
            before_mtime,
            "mtime restored"
        );
        assert_eq!(out.fingerprint.file_size, before.len() as u64);
        assert_eq!(
            out.fingerprint.file_modified,
            truncate_to_seconds(before_mtime).unwrap()
        );
        let read_back = read_metadata(&path).unwrap();
        assert_eq!(
            read_back.creation_time.unwrap().to_rfc3339(),
            "2024-07-04T12:00:00+00:00"
        );
        // Every fixed-width carrier the file has holds the same instant now:
        // this fixture is one video trak, so `mvhd` + `tkhd` + `mdhd`.
        let expected = u64::from(quicktime_seconds(edit.taken_at.unwrap()).unwrap());
        assert_eq!(date_fields(&after).len(), 3);
        assert_eq!(count_creation_times(&after, expected), 3);
    }

    #[test]
    fn patches_a_moov_at_the_end_and_every_carrier_of_a_two_track_file() {
        let (_dir, path) = temp_copy("moov_end.mp4", "test-data/test_video_moov_end.mp4");
        let before = fs::read(&path).unwrap();
        let edit = VideoMetadataEdit {
            taken_at: Some("2024-07-04T12:00:00Z".parse().unwrap()),
            ..Default::default()
        };
        let out = write_metadata(&path, &edit).unwrap();
        let after = fs::read(&path).unwrap();

        assert_eq!(after.len(), before.len());
        let (moov_off, moov_len) = locate_moov_in(&before).unwrap();
        assert!(
            moov_off > 1_000_000,
            "this fixture's moov sits behind a 1.7 MB mdat"
        );
        assert_eq!(&after[..moov_off], &before[..moov_off]);
        assert_eq!(
            &after[moov_off + moov_len..],
            &before[moov_off + moov_len..]
        );
        // `mvhd` + two traks × (`tkhd` + `mdhd`).
        let expected = u64::from(quicktime_seconds(edit.taken_at.unwrap()).unwrap());
        assert_eq!(date_fields(&after).len(), 5);
        assert_eq!(count_creation_times(&after, expected), 5);
        assert_eq!(out.fingerprint.file_size, after.len() as u64);
        assert_eq!(
            read_metadata(&path)
                .unwrap()
                .creation_time
                .unwrap()
                .to_rfc3339(),
            "2024-07-04T12:00:00+00:00"
        );
    }

    #[test]
    fn a_version_1_box_reads_and_takes_an_eight_byte_timestamp() {
        // A MOV that stores 64-bit times: the version word widens the creation
        // field to eight bytes on both sides. A patch that wrote the four bytes
        // of a version-0 field into one of these would leave the modification
        // field behind it corrupted.
        let creation =
            u64::from(quicktime_seconds("2023-06-15T10:00:00Z".parse().unwrap()).unwrap());
        let modification = creation + 7;
        let (_dir, path) =
            synthetic_version_1_time_file(&box_bytes(b"udta", &[]), creation, modification);
        let before = fs::read(&path).unwrap();
        let fields = date_fields(&before);
        assert_eq!(fields.len(), 3, "mvhd + tkhd + mdhd");
        assert!(
            fields.iter().all(|(_, _, width)| *width == 8),
            "a version-1 box declares a 64-bit field"
        );
        assert_eq!(
            read_metadata(&path)
                .unwrap()
                .creation_time
                .unwrap()
                .to_rfc3339(),
            "2023-06-15T10:00:00+00:00",
            "the exact instant is decoded from the 64-bit field"
        );

        let taken_at: DateTime<Utc> = "2024-07-04T12:00:00Z".parse().unwrap();
        let out = write_metadata(
            &path,
            &VideoMetadataEdit {
                taken_at: Some(taken_at),
                ..Default::default()
            },
        )
        .unwrap();
        let after = fs::read(&path).unwrap();

        assert_eq!(
            after.len(),
            before.len(),
            "eight bytes are written where eight bytes were"
        );
        let expected = u64::from(quicktime_seconds(taken_at).unwrap());
        for (_, offset, width) in date_fields(&before) {
            assert_eq!(width, 8);
            assert_eq!(read_field(&after, offset, width), expected);
            assert_eq!(
                read_field(&after, offset + width, width),
                modification,
                "the modification field behind the timestamp is intact"
            );
        }
        let creation_fields: Vec<_> = date_fields(&before)
            .into_iter()
            .map(|(_, offset, width)| offset..offset + width)
            .collect();
        for offset in differing_offsets(&before, &after) {
            assert!(
                creation_fields.iter().any(|field| field.contains(&offset)),
                "byte {offset} outside every creation field changed"
            );
        }
        assert_eq!(count_creation_times(&after, expected), 3);
        assert_eq!(out.fingerprint.file_size, after.len() as u64);
        assert_eq!(
            read_metadata(&path)
                .unwrap()
                .creation_time
                .unwrap()
                .to_rfc3339(),
            "2024-07-04T12:00:00+00:00"
        );
    }

    #[test]
    fn a_time_box_named_inside_the_item_list_is_not_a_creation_carrier() {
        // `is_container` parses every direct child of an `ilst` as a box, so an
        // item of type `mvhd` is walked as a movie header — and the writer,
        // which patched every time box it walked past, wrote the new instant
        // into the item's body+4: the type field of the very `data` box the
        // item is made of. No reader resolves a time box off its own path
        // (`mvhd_creation_time` reads a direct `moov` child), so those bytes
        // were not a carrier of anything: a save rewrote them, broke the item,
        // and no read ever looked at them again.
        let mut ilst = text_item_box(&DAY_BOX, b"2024-05-01 10:00:00");
        ilst.extend_from_slice(&text_item_box(b"mvhd", b"not a creation time"));
        let mut meta_body = vec![0u8, 0, 0, 0]; // version + flags
        meta_body.extend_from_slice(&mdta_hdlr());
        meta_body.extend_from_slice(&box_bytes(b"ilst", &ilst));
        let (_dir, path) =
            synthetic_file_with_udta(&box_bytes(b"udta", &box_bytes(b"meta", &meta_body)), 0);
        let before = fs::read(&path).unwrap();
        let (at, len) = ilst_item_span(&before, b"mvhd");

        write_metadata(&path, &date_edit()).unwrap();

        let after = fs::read(&path).unwrap();
        assert_eq!(
            &after[at..at + len],
            &before[at..at + len],
            "an item that borrows a time box's name is not a creation carrier"
        );
        assert_eq!(
            read_metadata(&path).unwrap().creation_date_text.as_deref(),
            Some("2024-07-04 12:00:00"),
            "the save still landed on the carrier the reader resolves"
        );
    }

    #[test]
    fn renders_the_new_instant_in_the_files_own_date_shape() {
        let new = "2024-07-04T12:00:00Z".parse().unwrap();
        assert_eq!(
            render_date_in_shape("2024-05-01T10:00:00+0200", new).unwrap(),
            b"2024-07-04T14:00:00+0200"
        );
        assert_eq!(
            render_date_in_shape("2024-05-01T10:00:00Z", new).unwrap(),
            b"2024-07-04T12:00:00Z"
        );
        assert_eq!(
            render_date_in_shape("2024-05-01T10:00:00.123+02:00", new).unwrap(),
            b"2024-07-04T14:00:00.000+02:00"
        );
        assert_eq!(
            render_date_in_shape("2024-05-01 10:00:00", new).unwrap(),
            b"2024-07-04 12:00:00"
        );
        assert_eq!(
            render_date_in_shape("2024-05-01", new).unwrap(),
            b"2024-07-04"
        );
        assert_eq!(render_date_in_shape("May 1st, 2024", new), None);
    }

    #[test]
    fn a_shorter_rendering_is_nul_padded_inside_the_existing_payload() {
        let mut slot = *b"2024-05-01T10:00:00+0200";
        write_payload_slot(&mut slot, b"2025-01-02T05:04:05+02");
        assert_eq!(&slot[..], b"2025-01-02T05:04:05+02\0\0");
    }

    #[test]
    fn a_date_save_updates_every_carrier_including_the_text_ones() {
        let (_dir, path) = temp_copy("keys.mp4", "test-data/test_video_quicktime_keys.mp4");
        let before = std::fs::read(&path).unwrap();
        let edit = VideoMetadataEdit {
            taken_at: Some("2025-01-02T03:04:05Z".parse().unwrap()),
            ..Default::default()
        };
        write_metadata(&path, &edit).unwrap();

        assert_eq!(
            read_metadata(&path)
                .unwrap()
                .creation_time
                .unwrap()
                .to_rfc3339(),
            "2025-01-02T03:04:05+00:00"
        );
        // The +0200-style carrier keeps its offset style and expresses the same instant:
        assert_eq!(
            read_metadata(&path).unwrap().creation_date_text.as_deref(),
            Some("2025-01-02T05:04:05+0200")
        );
        let after = std::fs::read(&path).unwrap();
        assert_eq!(after.len(), before.len());
        assert!(after.windows(24).any(|w| w == b"2025-01-02T05:04:05+0200"));
        assert!(!after.windows(24).any(|w| w == b"2024-05-01T10:00:00+0200"));
        // No box was resized: the data box that held the old string still declares the same size.
        assert_data_boxes_unchanged(&before, &after);
        // The neighbouring carriers in the same item list are asserted by value,
        // not by their (offset, size) pairs: a clobber of either would keep
        // every box the size it was.
        assert_eq!(
            read_metadata(&path).unwrap().location_iso6709.as_deref(),
            Some("+48.2082+016.3737/"),
            "the sibling location item is untouched"
        );
        assert!(
            after
                .windows(b"Lavf63.1.101".len())
                .any(|w| w == b"Lavf63.1.101"),
            "the sibling encoder item is untouched"
        );
    }

    #[test]
    fn an_unparsable_carrier_refuses_the_whole_save() {
        let (_dir, path) =
            synthetic_mp4_with_carrier(CarrierSpec::ItemType(*b"\xa9day"), b"May 1st, 2024", 0);
        let before = std::fs::read(&path).unwrap();
        let before_mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        let edit = VideoMetadataEdit {
            taken_at: Some("2025-01-02T03:04:05Z".parse().unwrap()),
            ..Default::default()
        };
        assert!(
            matches!(write_metadata(&path, &edit).unwrap_err(), Mp4MetadataError::Unrepresentable(t) if t == "\u{a9}day")
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            before_mtime,
            "a refused save does not even touch the file"
        );
    }

    #[test]
    fn every_date_carrier_ends_at_the_same_instant_in_its_own_style() {
        // Three carriers of the same file, each in a different shape: all of
        // them must end up at the new instant, none of them in a new style.
        let (_dir, path) = synthetic_multi_carrier_file();
        let before = fs::read(&path).unwrap();
        let taken_at: DateTime<Utc> = "2025-01-02T03:04:05Z".parse().unwrap();
        write_metadata(
            &path,
            &VideoMetadataEdit {
                taken_at: Some(taken_at),
                ..Default::default()
            },
        )
        .unwrap();

        let after = fs::read(&path).unwrap();
        assert_eq!(after.len(), before.len());
        for expected in [
            b"2025-01-02T05:04:05+0200".as_slice(),
            b"2025-01-02 03:04:05".as_slice(),
            b"2025-01-02T03:04:05.000Z".as_slice(),
        ] {
            assert!(
                after
                    .windows(expected.len())
                    .any(|window| window == expected),
                "missing {expected:?}"
            );
        }
        for stale in [
            b"2024-05-01T10:00:00+0200".as_slice(),
            b"2024-05-01 10:00:00".as_slice(),
            b"2024-05-01T10:00:00.500Z".as_slice(),
        ] {
            assert!(
                !after.windows(stale.len()).any(|window| window == stale),
                "stale {stale:?}"
            );
        }
        // The reader still resolves the mdta carrier first, and every
        // fixed-width carrier moved with the text ones.
        assert_eq!(
            read_metadata(&path).unwrap().creation_date_text.as_deref(),
            Some("2025-01-02T05:04:05+0200")
        );
        assert_eq!(
            count_creation_times(&after, u64::from(quicktime_seconds(taken_at).unwrap())),
            3
        );
        assert_data_boxes_unchanged(&before, &after);
    }

    #[test]
    fn a_carrier_named_by_the_second_mdta_key_is_rewritten_too() {
        // `creation_time` is the other mdta name for the same thing, and a
        // carrier under it has to move with the first one — fraction and `Z`
        // intact.
        let (_dir, path) = synthetic_mp4_with_carrier(
            CarrierSpec::MdtaKey(MDTA_CREATION_DATE_KEYS[1]),
            b"2024-05-01T10:00:00.5Z",
            0,
        );
        let before = fs::read(&path).unwrap();
        let taken_at: DateTime<Utc> = "2025-01-02T03:04:05Z".parse().unwrap();
        write_metadata(
            &path,
            &VideoMetadataEdit {
                taken_at: Some(taken_at),
                ..Default::default()
            },
        )
        .unwrap();

        let after = fs::read(&path).unwrap();
        assert_eq!(after.len(), before.len());
        assert_eq!(
            read_metadata(&path).unwrap().creation_date_text.as_deref(),
            Some("2025-01-02T03:04:05.0Z")
        );
        assert!(!after
            .windows(22)
            .any(|window| window == b"2024-05-01T10:00:00.5Z"));
        assert_eq!(
            count_creation_times(&after, u64::from(quicktime_seconds(taken_at).unwrap())),
            3
        );
        assert_data_boxes_unchanged(&before, &after);
    }

    #[test]
    fn a_date_only_carrier_keeps_its_date_only_shape() {
        let (_dir, path) =
            synthetic_mp4_with_carrier(CarrierSpec::ItemType(*b"\xa9day"), b"2024-05-01", 0);
        let before = fs::read(&path).unwrap();
        let taken_at: DateTime<Utc> = "2025-01-02T03:04:05Z".parse().unwrap();
        write_metadata(
            &path,
            &VideoMetadataEdit {
                taken_at: Some(taken_at),
                ..Default::default()
            },
        )
        .unwrap();

        let after = fs::read(&path).unwrap();
        assert_eq!(after.len(), before.len());
        assert_eq!(
            read_metadata(&path).unwrap().creation_date_text.as_deref(),
            Some("2025-01-02")
        );
        assert!(!after.windows(10).any(|window| window == b"2024-05-01"));
        assert_eq!(
            count_creation_times(&after, u64::from(quicktime_seconds(taken_at).unwrap())),
            3
        );
        assert_data_boxes_unchanged(&before, &after);
    }

    #[test]
    fn a_save_without_a_date_leaves_a_carrier_of_any_shape_alone() {
        // A save that names nothing renders no carrier, so the file comes out
        // byte-identical — whatever shapes the carriers it walked past have.
        // That is the whole claim of this test, and it is the only claim an
        // empty edit can carry: an edit with no date and no position never
        // reaches the per-shape rendering code, so pairing the shapes here
        // would walk four fixtures without entering a single one of them. The
        // per-shape walk is the next test.
        for (shape, payload) in [
            (CarrierSpec::ItemType(*b"\xa9day"), &b"May 1st, 2024"[..]),
            (CarrierSpec::ItemType(*b"\xa9day"), &b"2024-05-01"[..]),
            (CarrierSpec::ItemType(XYZ_BOX), &b"+48.2082+016.3737/"[..]),
            (
                CarrierSpec::MdtaKey(MDTA_CREATION_DATE_KEYS[0]),
                &b"2024-05-01T10:00:00+0200"[..],
            ),
        ] {
            let (_dir, path) = synthetic_mp4_with_carrier(shape, payload, 0);
            let before = fs::read(&path).unwrap();
            write_metadata(&path, &VideoMetadataEdit::default()).unwrap();
            assert_eq!(fs::read(&path).unwrap(), before, "{payload:?}");
        }
    }

    #[test]
    fn a_save_leaves_a_carrier_of_any_shape_in_its_own_shape() {
        // A save names a date or a position, and every other carrier in the
        // file has to come out of it in the shape the file already used: the
        // writer must not re-render a `©day` it walked past as an mdta pair, or
        // a `©xyz` in a form the file did not have, just because it saw it.
        //
        // Each shape below therefore sits next to a carrier of the OTHER kind.
        // A file that carries nothing else lets the writer walk past it without
        // deciding anything — it renders no carrier, so every assertion about
        // "the shape it was not asked about" would hold for free.
        let location = VideoMetadataEdit {
            latitude: Some(52.52),
            longitude: Some(13.405),
            ..Default::default()
        };
        for (shape, payload, date) in [
            // A text the writer cannot render at all, and a date-only text.
            (
                CarrierSpec::ItemType(DAY_BOX),
                &b"May 1st, 2024"[..],
                "May 1st, 2024",
            ),
            (
                CarrierSpec::ItemType(DAY_BOX),
                &b"2024-05-01"[..],
                "2024-05-01",
            ),
            (
                CarrierSpec::MdtaKey(MDTA_CREATION_DATE_KEYS[0]),
                &b"2024-05-01T10:00:00+0200"[..],
                "2024-05-01T10:00:00+0200",
            ),
        ] {
            let (_dir, path) = synthetic_mp4_with_carrier_and_counterpart(shape, payload, 0);
            write_metadata(&path, &location).unwrap();
            let after = read_metadata(&path).unwrap();
            assert_eq!(
                after.location_iso6709.as_deref(),
                // The file's own `©xyz` shape, solidus included: a reader that
                // dropped it would hand a caller a coordinate it cannot parse.
                Some("+52.5200+013.4050/"),
                "{payload:?}: the save has to land on the location carrier"
            );
            assert_eq!(
                after.creation_date_text.as_deref(),
                Some(date),
                "{payload:?}: the date carrier is the one the save did not name"
            );
        }

        let date = VideoMetadataEdit {
            taken_at: Some("2024-07-04T12:00:00Z".parse().unwrap()),
            ..Default::default()
        };
        for (shape, payload, saved) in [
            // A `©xyz` in the two decimal forms ffmpeg and Apple write.
            (
                CarrierSpec::ItemType(XYZ_BOX),
                &b"+48.2082+016.3737/"[..],
                "+48.2082+016.3737/",
            ),
            (
                CarrierSpec::ItemType(XYZ_BOX),
                &b"-33.8688+151.2093"[..],
                "-33.8688+151.2093",
            ),
        ] {
            let (_dir, path) = synthetic_mp4_with_carrier_and_counterpart(shape, payload, 0);
            write_metadata(&path, &date).unwrap();
            let after = read_metadata(&path).unwrap();
            assert_eq!(
                after.creation_date_text.as_deref(),
                Some("2024-07-04 12:00:00"),
                "{payload:?}: the save has to land on the date carrier"
            );
            assert_eq!(
                after.location_iso6709.as_deref(),
                Some(saved),
                "{payload:?}: the location carrier is the one the save did not name"
            );
        }
    }

    #[test]
    fn renders_coordinates_in_the_files_own_iso6709_shape() {
        assert_eq!(
            render_iso6709_in_shape("+48.2082+016.3737/", 52.52, 13.405).unwrap(),
            b"+52.5200+013.4050/"
        );
        assert_eq!(
            render_iso6709_in_shape("+48.2082+016.3737+150.00/", 52.52, 13.405).unwrap(),
            b"+52.5200+013.4050+150.00/"
        );
        assert_eq!(
            render_iso6709_in_shape("-33.8688+151.2093", -33.9, 151.3).unwrap(),
            b"-33.9000+151.3000"
        );
        // A narrower file width is widened, never kept: the value decides, the
        // writer pays for the extra bytes.
        assert_eq!(
            render_iso6709_in_shape("+8.2082+016.3737/", 52.52, 13.405).unwrap(),
            b"+52.5200+013.4050/"
        );
        // The stored integer width is kept as a floor, the sign is explicit and
        // a field without a fraction keeps none — the value is rounded to the
        // degree the file's own text can express.
        assert_eq!(
            render_iso6709_in_shape("-08.2082+16.3737", -0.5, 2.0).unwrap(),
            b"-00.5000+02.0000"
        );
        assert_eq!(
            render_iso6709_in_shape("+48+016/", 52.52, 13.405).unwrap(),
            b"+53+013/"
        );
        assert_eq!(render_iso6709_in_shape("garbage", 1.0, 2.0), None);
        for unparsable in [
            "",
            "+48.2082",
            "48.2082+016.3737",
            "+48.2082+016.3737//",
            "+48.2082+016.3737/+1.0",
            "+48.2082+016.3737+150.00+1.0",
            "+48.+016.3737",
        ] {
            assert_eq!(
                render_iso6709_in_shape(unparsable, 1.0, 2.0),
                None,
                "{unparsable}"
            );
        }
    }

    #[test]
    fn a_location_save_replaces_the_entry_without_duplicating_it() {
        /// The mdta key the fixture stores its location under: the whole entry
        /// text has to stay single.
        const KEY: &[u8] = b"com.apple.quicktime.location.ISO6709";
        let (_dir, path) = temp_copy("keys.mp4", "test-data/test_video_quicktime_keys.mp4");
        let before = fs::read(&path).unwrap();
        let before_len = std::fs::metadata(&path).unwrap().len();
        let edit = VideoMetadataEdit {
            latitude: Some(52.52),
            longitude: Some(13.405),
            ..Default::default()
        };
        write_metadata(&path, &edit).unwrap();

        assert_eq!(
            read_metadata(&path).unwrap().location_iso6709.as_deref(),
            Some("+52.5200+013.4050/")
        );
        // The save carries no date, so the file's own date text is left as it
        // was.
        assert_eq!(
            read_metadata(&path).unwrap().creation_date_text.as_deref(),
            Some("2024-05-01T10:00:00+0200")
        );
        let buf = fs::read(&path).unwrap();
        assert_eq!(
            buf.windows(KEY.len()).filter(|w| *w == KEY).count(),
            1,
            "the key entry is not duplicated"
        );
        assert_eq!(std::fs::metadata(&path).unwrap().len(), before_len);
        // The rendering is exactly as long as the text it replaces, so every
        // box the carrier lives in keeps its size.
        assert_data_boxes_unchanged(&before, &buf);
    }

    #[test]
    fn a_file_without_a_location_carrier_refuses_the_save() {
        let (_dir, path) = temp_copy("plain.mp4", "test-data/test_video_with_date.mp4");
        let before = fs::read(&path).unwrap();
        let edit = VideoMetadataEdit {
            latitude: Some(52.52),
            longitude: Some(13.405),
            ..Default::default()
        };
        assert!(matches!(
            write_metadata(&path, &edit).unwrap_err(),
            Mp4MetadataError::NoLocationCarrier
        ));
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn a_longer_render_uses_the_files_free_room_or_is_refused() {
        let edit = VideoMetadataEdit {
            latitude: Some(52.52),
            longitude: Some(13.405),
            ..Default::default()
        };
        let spec = || CarrierSpec::MdtaKey("com.apple.quicktime.location.ISO6709");
        // The stored value "+8.2082+016.3737/" is 17 bytes; the render
        // "+52.5200+013.4050/" is 18 — the latitude widens from one integer
        // digit to two — so the save needs one byte the slot does not have.
        let stored = b"+8.2082+016.3737/";
        let rendered = b"+52.5200+013.4050/";

        // (a) 16 bytes of payload behind the free box: the save succeeds, the
        //     file length is unchanged, and one free box of 23 bytes (15 of
        //     them payload) is left where the previous boxes were — the 24-byte
        //     box it replaces paid for the growth out of its own payload.
        let (_dir, path) = synthetic_mp4_with_carrier(spec(), stored, 16);
        let before_len = std::fs::metadata(&path).unwrap().len();
        write_metadata(&path, &edit).unwrap();
        assert_eq!(
            read_metadata(&path).unwrap().location_iso6709.as_deref(),
            Some("+52.5200+013.4050/")
        );
        let after = fs::read(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            before_len,
            "the file length is unchanged"
        );
        assert!(after.windows(rendered.len()).any(|w| w == rendered));
        assert_eq!(trailing_free_box_size(&after), 23);

        // (b) an 8-byte free box: the growth would leave 7 bytes behind, which
        //     is not a box, so the save is refused and the file is untouched.
        let (_dir2, path2) = synthetic_mp4_with_carrier(spec(), stored, 0);
        let before = fs::read(&path2).unwrap();
        let before_mtime = fs::metadata(&path2).unwrap().modified().unwrap();
        assert!(matches!(
            write_metadata(&path2, &edit).unwrap_err(),
            Mp4MetadataError::NoRoom(t) if t == "com.apple.quicktime.location.ISO6709"
        ));
        assert_eq!(fs::read(&path2).unwrap(), before);
        assert_eq!(
            fs::metadata(&path2).unwrap().modified().unwrap(),
            before_mtime,
            "a refused save does not even touch the file"
        );

        // (c) a 16-byte free box: the growth is paid for out of the whole box —
        //     its own header comes back with the bytes it wrapped — so the save
        //     lands and leaves 15 bytes of padding. Pricing the growth against
        //     the payload behind the header refused this, and refused it again
        //     one byte later.
        let (_dir3, path3) = synthetic_mp4_with_carrier(spec(), stored, 8);
        write_metadata(&path3, &edit).unwrap();
        assert_eq!(
            read_metadata(&path3).unwrap().location_iso6709.as_deref(),
            Some("+52.5200+013.4050/")
        );
        assert_eq!(trailing_free_box_size(&fs::read(&path3).unwrap()), 15);
    }

    #[test]
    fn the_filler_a_growth_leaves_is_what_prices_the_room() {
        // A `moov` whose only padding is a 24-byte `free` box. What a rebuild
        // emits is that box's 24 bytes less the growth, so a growth of 10 leaves
        // a valid 14-byte box and one of 24 leaves nothing at all: only a
        // remainder of 1..=7 bytes has no box to become. The payload behind the
        // header is not the budget — the header comes back with the bytes it
        // wrapped.
        let mut body = box_bytes(b"mvhd", &mvhd_body(0));
        body.extend_from_slice(&box_bytes(&XYZ_BOX, b"+8.2082+016.3737/"));
        body.extend_from_slice(&box_bytes(b"free", &[0u8; 16]));
        let original = box_bytes(b"moov", &body);
        let tree = parse_box_tree(&original, 0, original.len(), &[]).unwrap();
        let moov = &tree[0];
        let carrier = moov
            .children
            .iter()
            .find(|child| child.kind == XYZ_BOX)
            .unwrap();
        let payload = carrier.size - 8;
        let rebuilt = |growth: usize| {
            let edits = [MoovEdit {
                offset: carrier.offset + 8,
                replaced: payload,
                content: vec![b'x'; payload + growth],
                carrier: "test",
            }];
            rebuild_with_padding(&original, moov, &edits, growth as isize)
        };

        // 24 - 10 = 14 bytes of filler: the case a payload-only budget refused.
        let out = rebuilt(10).unwrap();
        assert_eq!(out.len(), original.len());
        assert_eq!(trailing_free_box_size(&out), 14);
        // 24 - 16 = 8: exactly the smallest box.
        assert_eq!(trailing_free_box_size(&rebuilt(16).unwrap()), 8);
        // 24 - 24 = 0: the padding is spent to the byte, nothing left to emit.
        let exact = rebuilt(24).unwrap();
        assert_eq!(exact.len(), original.len());
        assert_eq!(box_count(&exact, b"free"), 0, "no filler at all");
        // 24 - 17 = 7 and 24 - 25 = -1: no box fits, and there is no room.
        assert!(matches!(rebuilt(17), Err(Mp4MetadataError::NoRoom("test"))));
        assert!(matches!(rebuilt(25), Err(Mp4MetadataError::NoRoom("test"))));
    }

    #[test]
    fn the_ffmpeg_style_location_key_is_writable_too() {
        let (_dir, path) =
            synthetic_mp4_with_carrier(CarrierSpec::MdtaKey("location"), b"+48.2082+016.3737/", 0);
        let edit = VideoMetadataEdit {
            latitude: Some(-33.9),
            longitude: Some(151.3),
            ..Default::default()
        };
        write_metadata(&path, &edit).unwrap();
        assert_eq!(
            read_metadata(&path).unwrap().location_iso6709.as_deref(),
            Some("-33.9000+151.3000/")
        );
    }

    #[test]
    fn every_legacy_location_carrier_is_rewritten_in_place() {
        let (_dir, path) = synthetic_mp4_with_legacy_location_carriers(16);
        let before = fs::read(&path).unwrap();
        write_metadata(
            &path,
            &VideoMetadataEdit {
                latitude: Some(52.52),
                longitude: Some(13.405),
                ..Default::default()
            },
        )
        .unwrap();

        let after = fs::read(&path).unwrap();
        assert_eq!(after.len(), before.len(), "the file length is unchanged");
        // One `©xyz` item inside `ilst` and one direct `udta/©xyz` child: both
        // carry the new position, each in the shape its own text was written
        // in, and neither was duplicated.
        assert_eq!(box_count(&after, &XYZ_BOX), 2);
        assert_eq!(
            after
                .windows(18)
                .filter(|w| *w == b"+52.5200+013.4050/")
                .count(),
            1,
            "the four-decimal carrier keeps its solidus"
        );
        assert_eq!(
            after
                .windows(19)
                .filter(|w| *w == b"+52.52000+013.40500")
                .count(),
            1,
            "the five-decimal carrier keeps its width and has no solidus"
        );
        // The direct child grew by one byte; the 24-byte free box paid for it
        // and is re-emitted as one 23-byte box.
        assert_eq!(trailing_free_box_size(&after), 23);
        assert_eq!(
            read_metadata(&path).unwrap().location_iso6709.as_deref(),
            Some("+52.5200+013.4050/")
        );
    }

    #[test]
    fn a_quicktime_text_atom_carrier_reads_and_round_trips() {
        // ffmpeg writes the classic `udta` children as text atoms: a `u16` byte
        // count and a language code in front of the text. Reading the header as
        // part of the value made the position unparsable — the file was indexed
        // without its coordinates — and made every save on such a carrier
        // unrepresentable.
        let udta = box_bytes(
            b"udta",
            &[
                qt_text_atom(&DAY_BOX, b"2024-05-01T10:00:00+0200"),
                qt_text_atom(&XYZ_BOX, b"+48.2082+016.3737/"),
            ]
            .concat(),
        );
        let (_dir, path) = synthetic_file_with_udta(&udta, 0);
        let before = fs::read(&path).unwrap();
        let read = read_metadata(&path).unwrap();
        assert_eq!(
            read.creation_date_text.as_deref(),
            Some("2024-05-01T10:00:00+0200")
        );
        assert_eq!(read.location_iso6709.as_deref(), Some("+48.2082+016.3737/"));

        write_metadata(
            &path,
            &VideoMetadataEdit {
                taken_at: Some("2025-01-02T03:04:05Z".parse().unwrap()),
                latitude: Some(52.52),
                longitude: Some(13.405),
            },
        )
        .unwrap();

        let after = read_metadata(&path).unwrap();
        assert_eq!(
            after.creation_date_text.as_deref(),
            Some("2025-01-02T05:04:05+0200"),
            "the date atom is rewritten in its own shape"
        );
        assert_eq!(
            after.location_iso6709.as_deref(),
            Some("+52.5200+013.4050/"),
            "the position atom is rewritten in its own shape"
        );
        let bytes = fs::read(&path).unwrap();
        assert_eq!(bytes.len(), before.len(), "no box was resized");
        assert!(bytes.windows(24).any(|w| w == b"2025-01-02T05:04:05+0200"));
        assert!(bytes.windows(18).any(|w| w == b"+52.5200+013.4050/"));
    }

    #[test]
    fn a_grown_text_atom_carrier_keeps_its_byte_count_true() {
        // A text atom whose position needs one more byte than it holds: the
        // save pays for it out of the file's padding, and the atom's byte count
        // has to move with the text it names. The same save runs over a second
        // atom written in a different language, so the code in front of the text
        // is the atom's own rather than the one the other fixture happens to
        // carry.
        for language in [0x55c4u16, 0x0407] {
            let (_dir, path) = synthetic_file_with_udta(
                &box_bytes(
                    b"udta",
                    &qt_text_atom_in(&XYZ_BOX, b"+8.2082+16.3737/", language),
                ),
                16,
            );
            let before_len = std::fs::metadata(&path).unwrap().len();
            write_metadata(
                &path,
                &VideoMetadataEdit {
                    latitude: Some(52.52),
                    longitude: Some(13.405),
                    ..Default::default()
                },
            )
            .unwrap();

            assert_eq!(
                read_metadata(&path).unwrap().location_iso6709.as_deref(),
                Some("+52.5200+13.4050/")
            );
            let after = fs::read(&path).unwrap();
            assert_eq!(std::fs::metadata(&path).unwrap().len(), before_len);
            let header = [
                0u8,
                0x11,
                language.to_be_bytes()[0],
                language.to_be_bytes()[1],
            ];
            assert!(
                after.windows(4).any(|w| w == header),
                "the count counts the new text and the {language:#06x} language code survived"
            );
            assert_eq!(trailing_free_box_size(&after), 23);
        }
    }

    #[test]
    fn a_text_atom_carrier_stays_writable_after_a_save() {
        // Both text-atom layouts: the text alone behind the header, and the
        // text behind a NUL terminator the count has to cover. A save replaces
        // the whole payload and pads it back to the slot's size, so the count
        // it writes has to describe that padded payload — the reader tells the
        // two layouts apart by this word alone. A count short of the padding
        // demotes the atom to raw text, and the save after that one is
        // unrepresentable: the carrier is bricked by the save that used to
        // repair it.
        for terminated in [false, true] {
            let udta = box_bytes(
                b"udta",
                &[
                    qt_text_atom_terminated(&DAY_BOX, b"2024-05-01T10:00:00+0200", terminated),
                    qt_text_atom_terminated(&XYZ_BOX, b"+48.2082+016.3737/", terminated),
                ]
                .concat(),
            );
            let (_dir, path) = synthetic_file_with_udta(&udta, 0);
            assert_eq!(
                read_metadata(&path).unwrap().creation_date_text.as_deref(),
                Some("2024-05-01T10:00:00+0200"),
                "the fixture reads (terminated: {terminated})"
            );

            write_metadata(
                &path,
                &VideoMetadataEdit {
                    taken_at: Some("2025-01-02T03:04:05Z".parse().unwrap()),
                    latitude: Some(52.52),
                    longitude: Some(13.405),
                },
            )
            .unwrap();
            let after = read_metadata(&path).unwrap();
            assert_eq!(
                after.creation_date_text.as_deref(),
                Some("2025-01-02T05:04:05+0200"),
                "the first save rewrote the date atom (terminated: {terminated})"
            );
            assert_eq!(
                after.location_iso6709.as_deref(),
                Some("+52.5200+013.4050/"),
                "the first save rewrote the position atom (terminated: {terminated})"
            );

            write_metadata(
                &path,
                &VideoMetadataEdit {
                    taken_at: Some("2026-03-04T06:07:08Z".parse().unwrap()),
                    latitude: Some(48.85),
                    longitude: Some(2.35),
                },
            )
            .unwrap();
            let after = read_metadata(&path).unwrap();
            assert_eq!(
                after.creation_date_text.as_deref(),
                Some("2026-03-04T08:07:08+0200"),
                "the second save found the date atom still a text atom (terminated: {terminated})"
            );
            assert_eq!(
                after.location_iso6709.as_deref(),
                Some("+48.8500+002.3500/"),
                "the second save found the position atom still a text atom (terminated: {terminated})"
            );

            // The count names the whole padded payload: the text of the date
            // atom plus its terminator, so the reader's `count + 4` still lands
            // on the end of the payload.
            let count = u16::try_from("2026-03-04T08:07:08+0200".len() + usize::from(terminated))
                .unwrap()
                .to_be_bytes();
            assert!(
                fs::read(&path)
                    .unwrap()
                    .windows(4)
                    .any(|w| w == [count[0], count[1], 0x55, 0xc4]),
                "the rewritten date atom counts its padding (terminated: {terminated})"
            );
        }
    }

    #[test]
    fn an_empty_carrier_does_not_mask_the_one_below_it() {
        // An mdta location item whose `data` payload is all NULs reads as empty
        // text: it is not a position, and the file's own `udta/©xyz` child is.
        let mut meta_body = vec![0u8, 0, 0, 0]; // version + flags
        meta_body.extend_from_slice(&mdta_hdlr());
        meta_body.extend_from_slice(&keys_box(&[MDTA_LOCATION_KEYS[0]]));
        meta_body.extend_from_slice(&box_bytes(
            b"ilst",
            &text_item_box(&1u32.to_be_bytes(), b"\0"),
        ));
        let mut udta = box_bytes(b"meta", &meta_body);
        udta.extend_from_slice(&box_bytes(&XYZ_BOX, b"+48.2082+016.3737/"));
        let (_dir, path) = synthetic_file_with_udta(&box_bytes(b"udta", &udta), 0);
        let before = fs::read(&path).unwrap();
        assert_eq!(
            read_metadata(&path).unwrap().location_iso6709.as_deref(),
            Some("+48.2082+016.3737/")
        );

        // Saving to that file must land on the carrier that holds the value:
        // the empty item is skipped, not turned into a refusal over a value the
        // container "cannot express" — the file read correctly, so it has to
        // stay writable.
        write_metadata(
            &path,
            &VideoMetadataEdit {
                latitude: Some(48.25),
                longitude: Some(16.5),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            read_metadata(&path).unwrap().location_iso6709.as_deref(),
            Some("+48.2500+016.5000/")
        );
        let after = fs::read(&path).unwrap();
        let tree = parse_box_tree(&after, 0, after.len(), &[]).unwrap();
        let direct = find_path(
            tree.iter().find(|span| span.kind == *b"moov").unwrap(),
            &[*b"udta", XYZ_BOX],
        )
        .unwrap();
        let changed = differing_offsets(&before, &after);
        assert!(
            changed
                .iter()
                .all(|offset| (direct.offset..direct.offset + direct.size).contains(offset)),
            "the populated carrier took the value and the empty item was left alone: {changed:?}"
        );

        // A file whose only location carrier is an empty one has no location to
        // save to, and says exactly that — not that the value is one the
        // container cannot express.
        let mut meta_body = vec![0u8, 0, 0, 0]; // version + flags
        meta_body.extend_from_slice(&mdta_hdlr());
        meta_body.extend_from_slice(&keys_box(&[MDTA_LOCATION_KEYS[0]]));
        meta_body.extend_from_slice(&box_bytes(
            b"ilst",
            &text_item_box(&1u32.to_be_bytes(), b"\0"),
        ));
        let (_dir3, path3) =
            synthetic_file_with_udta(&box_bytes(b"udta", &box_bytes(b"meta", &meta_body)), 0);
        let before3 = fs::read(&path3).unwrap();
        assert_eq!(read_metadata(&path3).unwrap().location_iso6709, None);
        assert!(matches!(
            write_metadata(
                &path3,
                &VideoMetadataEdit {
                    latitude: Some(48.25),
                    longitude: Some(16.5),
                    ..Default::default()
                }
            )
            .unwrap_err(),
            Mp4MetadataError::NoLocationCarrier
        ));
        assert_eq!(fs::read(&path3).unwrap(), before3);

        // The same inside the key list: an empty higher-priority key does not
        // hide a lower-priority one that holds a date.
        let mut meta_body = vec![0u8, 0, 0, 0]; // version + flags
        meta_body.extend_from_slice(&mdta_hdlr());
        meta_body.extend_from_slice(&keys_box(&MDTA_CREATION_DATE_KEYS));
        let mut ilst = text_item_box(&1u32.to_be_bytes(), b"\0");
        ilst.extend_from_slice(&text_item_box(
            &2u32.to_be_bytes(),
            b"2024-05-01T10:00:00+0200",
        ));
        meta_body.extend_from_slice(&box_bytes(b"ilst", &ilst));
        let (_dir2, path2) =
            synthetic_file_with_udta(&box_bytes(b"udta", &box_bytes(b"meta", &meta_body)), 0);
        assert_eq!(
            read_metadata(&path2).unwrap().creation_date_text.as_deref(),
            Some("2024-05-01T10:00:00+0200")
        );
    }

    #[test]
    fn a_carrier_item_without_a_readable_data_box_is_no_carrier_at_all() {
        // The reader drops an `ilst` item whose `data` box is missing, and drops
        // one whose payload is too short to hold the type word and the version
        // flags in front of the text. The writer refused such an item instead,
        // which turned a file the reader found no position in into a save that
        // claimed the position was one the container cannot express. It is
        // skipped the way it is read, and a save with no carrier left says that.
        for (name, item) in [
            ("no data box", box_bytes(&XYZ_BOX, &[])),
            (
                "a data box with no room for the text",
                box_bytes(&XYZ_BOX, &box_bytes(b"data", &[])),
            ),
        ] {
            let meta_body = box_bytes(b"ilst", &item);
            let (_dir, path) =
                synthetic_file_with_udta(&box_bytes(b"udta", &box_bytes(b"meta", &meta_body)), 0);
            let before = fs::read(&path).unwrap();
            assert_eq!(
                read_metadata(&path).unwrap().location_iso6709,
                None,
                "{name}"
            );
            let err = write_metadata(
                &path,
                &VideoMetadataEdit {
                    latitude: Some(48.25),
                    longitude: Some(16.5),
                    ..Default::default()
                },
            )
            .unwrap_err();
            assert!(
                matches!(err, Mp4MetadataError::NoLocationCarrier),
                "{name}: {err}"
            );
            assert_eq!(
                fs::read(&path).unwrap(),
                before,
                "{name}: nothing was written"
            );
        }
    }

    #[test]
    fn a_loci_carrier_reads_as_iso6709_and_takes_a_save_in_place() {
        // ffmpeg's MP4 muxer writes a location tag as ISO/3GPP `udta/loci`, and
        // the scan-time remux (`-c copy -movflags +faststart`) hands a file
        // whose only carrier was a direct `©xyz` child to that muxer — so such
        // a file's position lives in the fixed-point pair alone. The pair is the
        // one ffmpeg 9.0.2's muxer writes for `+48.2082+016.3737/`.
        let udta = box_bytes(b"udta", &loci_box(b"", 0, 0x0030_354c, 0x0010_5faa));
        let (_dir, path) = synthetic_file_with_udta(&udta, 0);
        let before = fs::read(&path).unwrap();
        assert_eq!(
            read_metadata(&path).unwrap().location_iso6709.as_deref(),
            Some("+48.2082+016.3737/")
        );

        write_metadata(
            &path,
            &VideoMetadataEdit {
                latitude: Some(48.25),
                longitude: Some(16.5),
                ..Default::default()
            },
        )
        .unwrap();

        // The pair is 8 fixed bytes, so the save patches its two `i32`s in
        // place: no byte of the file moves, and no box changes size.
        let after = fs::read(&path).unwrap();
        assert_eq!(after.len(), before.len(), "the file length is unchanged");
        let pair = loci_pair_in(&before);
        let changed = differing_offsets(&before, &after);
        assert!(
            changed
                .iter()
                .all(|offset| (pair..pair + 8).contains(offset)),
            "only the horizontal pair is patched: {changed:?}"
        );
        let mut expected = 0x0010_8000u32.to_be_bytes().to_vec(); // longitude 16.5
        expected.extend_from_slice(&0x0030_4000u32.to_be_bytes()); // latitude 48.25
        assert_eq!(&after[pair..pair + 8], &expected[..]);
        assert_eq!(box_count(&after, b"loci"), 1, "the box is not duplicated");
        assert_eq!(
            read_metadata(&path).unwrap().location_iso6709.as_deref(),
            Some("+48.2500+016.5000/"),
            "the value round-trips through the reader"
        );
        // A name sits in front of the role byte, and the name is not part of
        // the pair: an exact position that is named still reads.
        let named = box_bytes(b"udta", &loci_box(b"Berlin", 0, 0x0030_354c, 0x0010_5faa));
        let (_dir2, path2) = synthetic_file_with_udta(&named, 0);
        assert_eq!(
            read_metadata(&path2).unwrap().location_iso6709.as_deref(),
            Some("+48.2082+016.3737/")
        );

        // The role byte says what the pair means, and only role 0 is a place.
        // A pair that names a room, a building, a floor, a postcode, an
        // intersection, an island, a street or an address is a region whose
        // extent the box does not store, so reading a longitude and a latitude
        // out of it puts a video somewhere its own metadata never claimed.
        for role in 1..=8u8 {
            let coarse = box_bytes(
                b"udta",
                &loci_box(b"Berlin", role, 0x0030_354c, 0x0010_5faa),
            );
            let (_dir, path) = synthetic_file_with_udta(&coarse, 0);
            let before = fs::read(&path).unwrap();
            assert_eq!(
                read_metadata(&path).unwrap().location_iso6709,
                None,
                "role {role} is not an exact position"
            );

            // And a save does not land in it either: a file whose only
            // location is a coarse `loci` is refused the position rather than
            // having the region overwritten with a point, which would be a
            // loss the user never asked for.
            assert!(
                matches!(
                    write_metadata(
                        &path,
                        &VideoMetadataEdit {
                            latitude: Some(48.25),
                            longitude: Some(16.5),
                            ..Default::default()
                        },
                    )
                    .unwrap_err(),
                    Mp4MetadataError::NoLocationCarrier
                ),
                "role {role} must not take a save"
            );
            assert_eq!(
                fs::read(&path).unwrap(),
                before,
                "role {role}: nothing written"
            );
        }

        // A pair outside the valid ranges is reported verbatim, the way the text
        // carriers' values are: validating the position is the caller's
        // business, and the carrier is not silently dropped.
        let wild = box_bytes(b"udta", &loci_box(b"", 0, i32::MAX, i32::MIN));
        let (_dir3, path3) = synthetic_file_with_udta(&wild, 0);
        assert_eq!(
            read_metadata(&path3).unwrap().location_iso6709.as_deref(),
            Some("+32768.0000-32768.0000/")
        );
    }

    #[test]
    fn a_loci_pair_that_cannot_hold_the_value_is_refused() {
        let udta = box_bytes(b"udta", &loci_box(b"", 0, 0x0030_354c, 0x0010_5faa));
        let (_dir, path) = synthetic_file_with_udta(&udta, 0);
        let buf = fs::read(&path).unwrap();
        let tree = parse_box_tree(&buf, 0, buf.len(), &[]).unwrap();
        let moov = tree.iter().find(|span| span.kind == *b"moov").unwrap();
        let loci = find_path(moov, &[*b"udta", *b"loci"]).unwrap();

        // The pair is an `i32` of 1/65536 degree: a value the 16.16 format
        // cannot hold is refused instead of being wrapped into it.
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1.0e9] {
            assert_eq!(fixed_point_16_16(value), None, "{value}");
        }
        let err = loci_edit(&buf, loci, 1.0e9, 16.0).err().unwrap();
        assert!(
            matches!(err, Mp4MetadataError::Unrepresentable("loci")),
            "{err}"
        );
        // A position the pair can hold is never refused.
        assert!(loci_edit(&buf, loci, 48.2082, 16.3737).unwrap().is_some());

        // A `loci` box too short to hold a pair is not a carrier: the reader
        // answers no position, and the writer skips it instead of refusing over
        // a box the file never filled in.
        let (_dir2, path2) =
            synthetic_file_with_udta(&box_bytes(b"udta", &box_bytes(b"loci", &[0u8; 4])), 0);
        assert_eq!(read_metadata(&path2).unwrap().location_iso6709, None);
        let before = fs::read(&path2).unwrap();
        assert!(matches!(
            write_metadata(
                &path2,
                &VideoMetadataEdit {
                    latitude: Some(48.25),
                    longitude: Some(16.5),
                    ..Default::default()
                }
            )
            .unwrap_err(),
            Mp4MetadataError::NoLocationCarrier
        ));
        assert_eq!(fs::read(&path2).unwrap(), before);
    }

    #[test]
    fn an_empty_date_carrier_does_not_block_the_save() {
        // An mdta date item whose payload is all NULs above a populated `©day`
        // item: the reader takes the populated one, so a save has to write it
        // rather than refuse over the slot the file never filled.
        let mut meta_body = vec![0u8, 0, 0, 0]; // version + flags
        meta_body.extend_from_slice(&mdta_hdlr());
        meta_body.extend_from_slice(&keys_box(&[MDTA_CREATION_DATE_KEYS[0]]));
        let mut ilst = text_item_box(&1u32.to_be_bytes(), b"\0");
        ilst.extend_from_slice(&text_item_box(&DAY_BOX, b"2024-05-01T10:00:00+0200"));
        meta_body.extend_from_slice(&box_bytes(b"ilst", &ilst));
        let (_dir, path) =
            synthetic_file_with_udta(&box_bytes(b"udta", &box_bytes(b"meta", &meta_body)), 0);
        assert_eq!(
            read_metadata(&path).unwrap().creation_date_text.as_deref(),
            Some("2024-05-01T10:00:00+0200")
        );

        write_metadata(
            &path,
            &VideoMetadataEdit {
                taken_at: Some("2025-01-02T03:04:05Z".parse().unwrap()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            read_metadata(&path).unwrap().creation_date_text.as_deref(),
            Some("2025-01-02T05:04:05+0200"),
            "the populated carrier took the instant"
        );
    }

    #[test]
    fn a_date_edit_on_a_container_with_no_date_carrier_is_refused() {
        // A container that carries a position and no date at all: no
        // `mvhd`/`tkhd`/`mdhd` to hold the instant and no text date carrier
        // either. Writing the date anyway would hand back a save that reports
        // an instant the file does not hold, so the save is refused the way a
        // container without a location carrier is — and the position the file
        // does carry stays writable, which is what shows the refusal is about
        // the date and not about the container.
        let udta = box_bytes(b"udta", &qt_text_atom(&XYZ_BOX, b"+48.2082+016.3737/"));
        let (_dir, path) = synthetic_file_with_moov(&udta, MoovSizeForm::Wide32);
        let before = fs::read(&path).unwrap();
        let before_mtime = fs::metadata(&path).unwrap().modified().unwrap();

        assert!(matches!(
            write_metadata(
                &path,
                &VideoMetadataEdit {
                    taken_at: Some("2025-01-02T03:04:05Z".parse().unwrap()),
                    ..Default::default()
                }
            )
            .unwrap_err(),
            Mp4MetadataError::Unrepresentable(carrier) if carrier == DATE_CARRIER
        ));
        assert_eq!(
            fs::read(&path).unwrap(),
            before,
            "the refusal wrote nothing"
        );
        assert_eq!(
            fs::metadata(&path).unwrap().modified().unwrap(),
            before_mtime
        );

        write_metadata(
            &path,
            &VideoMetadataEdit {
                latitude: Some(52.52),
                longitude: Some(13.405),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            read_metadata(&path).unwrap().location_iso6709.as_deref(),
            Some("+52.5200+013.4050/")
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_edit_that_names_no_value_leaves_the_file_alone() {
        use std::os::unix::fs::PermissionsExt;

        // An empty request is what a form sends when the user saves without
        // touching a field: there is no value to render, so the file is not
        // opened for writing at all. A read-only file is what proves that —
        // the same file refuses a real edit and accepts this one.
        let (_dir, path) = temp_copy("clip.mp4", "test-data/test_video_with_date.mp4");
        let before = fs::read(&path).unwrap();
        let before_mtime = fs::metadata(&path).unwrap().modified().unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        // Root, and anything holding CAP_DAC_OVERRIDE, may write a read-only
        // file regardless, so the permission bit has to be the deciding factor
        // for this test to mean anything.
        if fs::OpenOptions::new().write(true).open(&path).is_ok() {
            eprintln!("Skipping read-only test: this user may write a 0444 file");
            return;
        }

        let write = write_metadata(&path, &VideoMetadataEdit::default()).unwrap();
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(
            fs::metadata(&path).unwrap().modified().unwrap(),
            before_mtime,
            "the modification time was not even given back to itself"
        );
        assert_eq!(write.fingerprint.file_size, before.len() as u64);
        assert_eq!(
            write.fingerprint.file_modified,
            truncate_to_seconds(before_mtime).unwrap(),
            "the fingerprint reports the file as it was"
        );
        assert!(matches!(
            write_metadata(
                &path,
                &VideoMetadataEdit {
                    taken_at: Some("2024-07-04T12:00:00Z".parse().unwrap()),
                    ..Default::default()
                }
            )
            .unwrap_err(),
            Mp4MetadataError::ReadOnly(_)
        ));
    }

    #[test]
    fn a_second_item_of_a_kind_carries_the_value_when_the_first_is_empty() {
        // The same mdta kind twice: the first item's payload is all NULs, the
        // second holds the position. The lookup consults every item of the kind
        // instead of stopping at the first one, the way the write side walks
        // them.
        let mut meta_body = vec![0u8, 0, 0, 0]; // version + flags
        meta_body.extend_from_slice(&mdta_hdlr());
        meta_body.extend_from_slice(&keys_box(&[MDTA_LOCATION_KEYS[0]]));
        let mut ilst = text_item_box(&1u32.to_be_bytes(), b"\0");
        ilst.extend_from_slice(&text_item_box(&1u32.to_be_bytes(), b"+48.2082+016.3737/"));
        meta_body.extend_from_slice(&box_bytes(b"ilst", &ilst));
        let (_dir, path) =
            synthetic_file_with_udta(&box_bytes(b"udta", &box_bytes(b"meta", &meta_body)), 0);
        assert_eq!(
            read_metadata(&path).unwrap().location_iso6709.as_deref(),
            Some("+48.2082+016.3737/")
        );

        // Both items are carriers to the writer, so the file is writable: the
        // empty one is skipped and the populated one takes the value.
        write_metadata(
            &path,
            &VideoMetadataEdit {
                latitude: Some(48.25),
                longitude: Some(16.5),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            read_metadata(&path).unwrap().location_iso6709.as_deref(),
            Some("+48.2500+016.5000/")
        );
    }

    #[test]
    fn a_second_legacy_item_carries_the_value_when_the_first_is_empty() {
        // The same *legacy* kind twice: the first `©xyz` item's payload is all
        // NULs, the second holds the position. The lookup walks every item of
        // the kind, the way the write side does, instead of answering with the
        // first one — which would report no location for a file that carries
        // one, and let a save claim a pair `read_metadata` never resolves.
        let mut meta_body = vec![0u8, 0, 0, 0]; // version + flags
        meta_body.extend_from_slice(&mdta_hdlr());
        let mut ilst = text_item_box(&XYZ_BOX, b"\0");
        ilst.extend_from_slice(&text_item_box(&XYZ_BOX, b"+48.2082+016.3737/"));
        meta_body.extend_from_slice(&box_bytes(b"ilst", &ilst));
        let (_dir, path) =
            synthetic_file_with_udta(&box_bytes(b"udta", &box_bytes(b"meta", &meta_body)), 16);
        let before = fs::read(&path).unwrap();
        assert_eq!(
            read_metadata(&path).unwrap().location_iso6709.as_deref(),
            Some("+48.2082+016.3737/")
        );

        // The populated item is a carrier to the writer while the empty one is
        // skipped, so the value must survive the read-back.
        write_metadata(
            &path,
            &VideoMetadataEdit {
                latitude: Some(48.25),
                longitude: Some(16.5),
                ..Default::default()
            },
        )
        .unwrap();
        let after = fs::read(&path).unwrap();
        assert_eq!(
            read_metadata(&path).unwrap().location_iso6709.as_deref(),
            Some("+48.2500+016.5000/")
        );
        let tree = parse_box_tree(&after, 0, after.len(), &[]).unwrap();
        let moov = tree.iter().find(|span| span.kind == *b"moov").unwrap();
        let populated = find_path(moov, &[*b"udta", META_BOX, *b"ilst"])
            .unwrap()
            .children
            .last()
            .unwrap();
        let changed = differing_offsets(&before, &after);
        assert!(
            changed.iter().all(
                |offset| (populated.offset..populated.offset + populated.size).contains(offset)
            ),
            "only the populated item took the value: {changed:?}"
        );

        // The same for the legacy date kind: an empty `©day` item in front of a
        // populated one must not hide the date from the reader, or the save
        // would land the instant in a carrier the reader never resolves.
        let mut meta_body = vec![0u8, 0, 0, 0]; // version + flags
        meta_body.extend_from_slice(&mdta_hdlr());
        let mut ilst = text_item_box(&DAY_BOX, b"\0");
        ilst.extend_from_slice(&text_item_box(&DAY_BOX, b"2024-05-01T10:00:00+0200"));
        meta_body.extend_from_slice(&box_bytes(b"ilst", &ilst));
        let (_dir2, path2) =
            synthetic_file_with_udta(&box_bytes(b"udta", &box_bytes(b"meta", &meta_body)), 0);
        assert_eq!(
            read_metadata(&path2).unwrap().creation_date_text.as_deref(),
            Some("2024-05-01T10:00:00+0200")
        );
        write_metadata(
            &path2,
            &VideoMetadataEdit {
                taken_at: Some("2025-01-02T03:04:05Z".parse().unwrap()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            read_metadata(&path2).unwrap().creation_date_text.as_deref(),
            Some("2025-01-02T05:04:05+0200"),
            "the populated legacy date item took the instant"
        );
    }

    #[test]
    fn a_doubled_key_names_every_position_it_carries() {
        // `keys` names the location key and the date key twice each, with the
        // item at the first position empty and the item at the later position
        // populated. The reader consults every position naming the name — the
        // writer patches every one of them, so a save must not land in a
        // carrier `read_metadata` cannot reach.
        let mut meta_body = vec![0u8, 0, 0, 0]; // version + flags
        meta_body.extend_from_slice(&mdta_hdlr());
        meta_body.extend_from_slice(&keys_box(&[
            MDTA_LOCATION_KEYS[0],
            MDTA_LOCATION_KEYS[0],
            MDTA_CREATION_DATE_KEYS[0],
            MDTA_CREATION_DATE_KEYS[0],
        ]));
        let mut ilst = text_item_box(&1u32.to_be_bytes(), b"\0");
        ilst.extend_from_slice(&text_item_box(&2u32.to_be_bytes(), b"+48.2082+016.3737/"));
        ilst.extend_from_slice(&text_item_box(&3u32.to_be_bytes(), b"\0"));
        ilst.extend_from_slice(&text_item_box(
            &4u32.to_be_bytes(),
            b"2024-05-01T10:00:00+0200",
        ));
        meta_body.extend_from_slice(&box_bytes(b"ilst", &ilst));
        let (_dir, path) =
            synthetic_file_with_udta(&box_bytes(b"udta", &box_bytes(b"meta", &meta_body)), 0);
        let read = read_metadata(&path).unwrap();
        assert_eq!(read.location_iso6709.as_deref(), Some("+48.2082+016.3737/"));
        assert_eq!(
            read.creation_date_text.as_deref(),
            Some("2024-05-01T10:00:00+0200")
        );

        // The same file the write side targets: the item at the later position
        // takes both values, and both survive the read-back.
        write_metadata(
            &path,
            &VideoMetadataEdit {
                taken_at: Some("2025-01-02T03:04:05Z".parse().unwrap()),
                latitude: Some(48.25),
                longitude: Some(16.5),
            },
        )
        .unwrap();
        let read_back = read_metadata(&path).unwrap();
        assert_eq!(
            read_back.location_iso6709.as_deref(),
            Some("+48.2500+016.5000/")
        );
        assert_eq!(
            read_back.creation_date_text.as_deref(),
            Some("2025-01-02T05:04:05+0200")
        );
    }

    #[test]
    fn a_key_named_in_hundreds_of_positions_is_read_in_one_pass() {
        // A container may name one key in every position of `keys` and fill
        // every position with an item. Reading it by rescanning the item list
        // once per named position costs items × positions on every read of
        // every video on every scan — for this file a quarter of a million
        // comparisons, for a file ten times this size two and a half million,
        // all of them to find one value. The index is built in a single pass
        // over the items, and the lookup is a hash probe per position, so the
        // cost is bounded by the file rather than by its square.
        const POSITIONS: usize = 512;
        let names = vec![MDTA_CREATION_DATE_KEYS[0]; POSITIONS];
        let mut meta_body = vec![0u8, 0, 0, 0]; // version + flags
        meta_body.extend_from_slice(&mdta_hdlr());
        meta_body.extend_from_slice(&keys_box(&names));
        let mut ilst = Vec::new();
        for position in 1..=POSITIONS {
            let index = u32::try_from(position).unwrap().to_be_bytes();
            // Only the last position holds the value: every earlier item is the
            // empty duplicate the lookup has to look through.
            let text: &[u8] = if position == POSITIONS {
                b"2024-05-01T10:00:00+0200"
            } else {
                b"\0"
            };
            ilst.extend_from_slice(&text_item_box(&index, text));
        }
        meta_body.extend_from_slice(&box_bytes(b"ilst", &ilst));
        let (_dir, path) =
            synthetic_file_with_udta(&box_bytes(b"udta", &box_bytes(b"meta", &meta_body)), 0);

        assert_eq!(
            read_metadata(&path).unwrap().creation_date_text.as_deref(),
            Some("2024-05-01T10:00:00+0200"),
            "the value is found at the last of {POSITIONS} positions naming it"
        );

        // And the file it read was walked once. The count is taken from the
        // read that just ran, not from an index the test builds for itself and
        // not from a clock: a lookup that went back to rescanning the items
        // once per named position would read the same value above while
        // walking nothing here, and a wall clock would only say the work
        // finished.
        assert_eq!(
            take_items_walked(),
            POSITIONS,
            "the read walked the item list once, not once per named position"
        );
    }

    #[test]
    fn a_key_named_in_hundreds_of_positions_is_written_in_one_pass() {
        // The same container as the read path's one-pass case, saved rather than
        // read: the writer resolved its carriers the same way — the whole item
        // list rescanned once per named position — and a save pays it twice
        // over, because `restore_region` renders the same region again to
        // check what the first render wrote. For this file that is a quarter of
        // a million comparisons to land one instant, and a file ten times this
        // size two and a half million. The index is built in a single pass over
        // the items and shared by both collectors, so the cost is bounded by
        // the file rather than by its square.
        const POSITIONS: usize = 512;
        let names = vec![MDTA_CREATION_DATE_KEYS[0]; POSITIONS];
        let mut meta_body = vec![0u8, 0, 0, 0]; // version + flags
        meta_body.extend_from_slice(&mdta_hdlr());
        meta_body.extend_from_slice(&keys_box(&names));
        let mut ilst = Vec::new();
        for position in 1..=POSITIONS {
            let index = u32::try_from(position).unwrap().to_be_bytes();
            ilst.extend_from_slice(&text_item_box(&index, b"2024-05-01T10:00:00+0200"));
        }
        meta_body.extend_from_slice(&box_bytes(b"ilst", &ilst));
        let (_dir, path) =
            synthetic_file_with_udta(&box_bytes(b"udta", &box_bytes(b"meta", &meta_body)), 0);

        write_metadata(
            &path,
            &VideoMetadataEdit {
                taken_at: Some("2025-01-02T03:04:05Z".parse().unwrap()),
                ..Default::default()
            },
        )
        .unwrap();

        // Every one of the {POSITIONS} positions naming the key was still
        // written: replacing the rescan must not have replaced a carrier with
        // it. Counting the item texts beats reading one back, because the
        // reader answers with the first carrier it resolves — which is exactly
        // the carrier a rescan would still have reached.
        let after = fs::read(&path).unwrap();
        let tree = parse_box_tree(&after, 0, after.len(), &[]).unwrap();
        let moov = tree.iter().find(|span| span.kind == *b"moov").unwrap();
        let written = ilst_items(&after, moov)
            .iter()
            .filter(|item| item.text == "2025-01-02T05:04:05+0200")
            .count();
        assert_eq!(
            written, POSITIONS,
            "every named position took the new instant"
        );

        // And the resolution cost one pass over the item list. The count comes
        // from the save that just ran and is taken where a collector reaches an
        // item at all — the index's own door — not from a clock and not from an
        // index the test builds for itself. A resolution that went back to
        // walking the item list per named position would produce the very same
        // file above while handing itself {POSITIONS}² items, and a wall clock
        // would only say the work finished.
        assert_eq!(
            take_carriers_resolved(),
            POSITIONS,
            "the save resolved {POSITIONS} carriers, not {POSITIONS}² item visits"
        );
    }

    #[test]
    fn two_populated_items_of_one_name_are_both_carriers() {
        // The same doubled key with BOTH of its items populated, which is the
        // case the doubled-name test cannot reach: with the first item empty, a
        // writer that only ever patched the first one still produces a file
        // whose read-back is right. Here nothing may be left behind. The check
        // is by byte, because "the write landed in the carriers" and "every
        // carrier was written" are two different claims: the first keeps a
        // rewrite from spilling out of the `ilst`, the second catches the item
        // the writer dropped.
        let mut meta_body = vec![0u8, 0, 0, 0]; // version + flags
        meta_body.extend_from_slice(&mdta_hdlr());
        meta_body.extend_from_slice(&keys_box(&[
            MDTA_LOCATION_KEYS[0],
            MDTA_LOCATION_KEYS[0],
            MDTA_CREATION_DATE_KEYS[0],
            MDTA_CREATION_DATE_KEYS[0],
        ]));
        let mut ilst = text_item_box(&1u32.to_be_bytes(), b"+48.2082+016.3737/");
        ilst.extend_from_slice(&text_item_box(&2u32.to_be_bytes(), b"+49.3717+017.2082/"));
        ilst.extend_from_slice(&text_item_box(
            &3u32.to_be_bytes(),
            b"2020-01-02T03:04:05+0200",
        ));
        ilst.extend_from_slice(&text_item_box(
            &4u32.to_be_bytes(),
            b"2021-02-03T04:05:06+0200",
        ));
        meta_body.extend_from_slice(&box_bytes(b"ilst", &ilst));
        let (_dir, path) =
            synthetic_file_with_udta(&box_bytes(b"udta", &box_bytes(b"meta", &meta_body)), 0);
        let before = fs::read(&path).unwrap();

        write_metadata(
            &path,
            &VideoMetadataEdit {
                taken_at: Some("2025-01-02T03:04:05Z".parse().unwrap()),
                latitude: Some(48.25),
                longitude: Some(16.5),
            },
        )
        .unwrap();
        let after = fs::read(&path).unwrap();
        assert_eq!(
            after.len(),
            before.len(),
            "both items hold the shape the value was rendered into, so the \
             carriers keep their length"
        );

        let items = || {
            let tree = parse_box_tree(&before, 0, before.len(), &[]).unwrap();
            let moov = tree.iter().find(|span| span.kind == *b"moov").unwrap();
            find_path(moov, &[*b"udta", META_BOX, *b"ilst"])
                .unwrap()
                .children
                .iter()
                .map(|item| item.offset..item.offset + item.size)
                .collect::<Vec<_>>()
        };
        let carriers = items();
        assert_eq!(
            carriers.len(),
            4,
            "the fixture has one item per key position"
        );
        // The header time fields are the one thing a date save writes besides
        // the carriers, so they are named here: every changed byte has to be in
        // a carrier or in one of them, and every carrier has to be in the
        // change. A writer that patched one item of a pair passes the second
        // assertion only for the item it reached, and one that rewrote a whole
        // box it did not have to fails the first.
        let headers = date_fields(&before)
            .into_iter()
            .map(|(_, at, width)| at..at + 2 * width)
            .collect::<Vec<_>>();
        let changed = differing_offsets(&before, &after);
        assert!(!changed.is_empty(), "the save has to change something");
        assert!(
            changed.iter().all(|offset| {
                carriers
                    .iter()
                    .chain(&headers)
                    .any(|span| span.contains(offset))
            }),
            "the write landed outside the carriers: {changed:?}"
        );
        assert!(
            carriers
                .iter()
                .all(|item| item.clone().any(|offset| changed.contains(&offset))),
            "an item is still holding the value the file came in with"
        );

        let read_back = read_metadata(&path).unwrap();
        assert_eq!(
            read_back.location_iso6709.as_deref(),
            Some("+48.2500+016.5000/")
        );
        assert_eq!(
            read_back.creation_date_text.as_deref(),
            Some("2025-01-02T05:04:05+0200")
        );
    }

    #[test]
    fn every_direct_udta_child_of_a_name_is_a_carrier() {
        // A container may hold the same direct `udta` child twice. The reader
        // stopped at the first one it found, so a file whose first copy held
        // nothing read as carrying no date at all, and the writer patched only
        // the first copy — which meant a save to such a file left the second
        // copy stale, and the read that followed it reported the value the save
        // was supposed to have replaced. Every copy is a carrier, and the save
        // reaches all of them.
        let mut udta_body = box_bytes(&DAY_BOX, b"");
        udta_body.extend_from_slice(&box_bytes(&DAY_BOX, b"2020-01-02T10:00:00+0200"));
        udta_body.extend_from_slice(&box_bytes(&DAY_BOX, b"2021-02-03 10:00:00.500Z"));
        let (_dir, path) = synthetic_file_with_udta(&box_bytes(b"udta", &udta_body), 0);
        assert_eq!(
            read_metadata(&path).unwrap().creation_date_text.as_deref(),
            Some("2020-01-02T10:00:00+0200"),
            "the empty copy ahead of the populated one is not a carrier"
        );

        write_metadata(
            &path,
            &VideoMetadataEdit {
                taken_at: Some("2024-05-01T08:00:00Z".parse().unwrap()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            read_metadata(&path).unwrap().creation_date_text.as_deref(),
            Some("2024-05-01T10:00:00+0200")
        );

        // Both populated copies were written, each in its own shape: neither the
        // value the file came in with is left anywhere in it.
        let after = fs::read(&path).unwrap();
        [
            b"2020-01-02T10:00:00+0200".as_slice(),
            b"2021-02-03 10:00:00.500Z",
        ]
        .iter()
        .for_each(|stale| {
            assert!(
                !after.windows(stale.len()).any(|window| window == *stale),
                "{} is still in the file",
                String::from_utf8_lossy(stale)
            );
        });
    }

    #[test]
    fn a_rebuild_puts_the_filler_before_a_trailing_tail() {
        // A container may end with 1..=7 bytes behind its last child that are not
        // a box — the walk stops in front of them. A rebuild has to emit its
        // filler in front of those bytes, or the filler is no longer the last
        // child and the walk over the rebuild reads the tail where the padding
        // belongs.
        let mut body = box_bytes(b"mvhd", &mvhd_body(0));
        body.extend_from_slice(&box_bytes(&XYZ_BOX, b"+8.2082+016.3737/"));
        body.extend_from_slice(&box_bytes(b"free", &[0u8; 16]));
        body.extend_from_slice(&[0xFF; 4]);
        let original = box_bytes(b"moov", &body);
        let tree = parse_box_tree(&original, 0, original.len(), &[]).unwrap();
        let moov = &tree[0];
        assert_eq!(moov.size, original.len());
        let carrier = moov
            .children
            .iter()
            .find(|child| child.kind == XYZ_BOX)
            .unwrap();
        let payload = carrier.size - 8;
        let edits = [MoovEdit {
            offset: carrier.offset + 8,
            replaced: payload,
            content: vec![b'x'; payload + 1],
            carrier: "test",
        }];

        let out = rebuild_with_padding(&original, moov, &edits, 1).unwrap();
        assert_eq!(out.len(), original.len(), "the moov keeps its length");
        // The rebuild parses, and the filler is the last child the walk finds.
        let rebuilt = parse_box_tree(&out, 0, out.len(), &[]).unwrap();
        let last = rebuilt[0].children.last().unwrap();
        assert_eq!(last.kind, *b"free");
        assert_eq!(last.size, 23);
        assert_eq!(trailing_free_box_size(&out), 23);
        assert_eq!(
            &out[out.len() - 4..],
            &[0xFF; 4],
            "the trailing bytes are still behind the filler"
        );
    }

    #[test]
    fn a_skip_box_is_padding_too() {
        // The same synthetic file with its trailing `free` box declared `skip`:
        // both spellings are room a save may spend.
        let (_dir, path) = synthetic_mp4_with_carrier(
            CarrierSpec::MdtaKey(MDTA_LOCATION_KEYS[0]),
            b"+8.2082+016.3737/",
            16,
        );
        let mut bytes = fs::read(&path).unwrap();
        let tree = parse_box_tree(&bytes, 0, bytes.len(), &[]).unwrap();
        let moov = tree.iter().find(|span| span.kind == *b"moov").unwrap();
        let padding = moov
            .children
            .iter()
            .rfind(|child| is_padding(&child.kind))
            .expect("the fixture has a padding box");
        bytes[padding.offset + 4..padding.offset + 8].copy_from_slice(b"skip");
        fs::write(&path, &bytes).unwrap();
        let before_len = std::fs::metadata(&path).unwrap().len();

        write_metadata(
            &path,
            &VideoMetadataEdit {
                latitude: Some(52.52),
                longitude: Some(13.405),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), before_len);
        assert_eq!(
            read_metadata(&path).unwrap().location_iso6709.as_deref(),
            Some("+52.5200+013.4050/")
        );
        assert_eq!(
            trailing_free_box_size(&fs::read(&path).unwrap()),
            23,
            "the padding is re-emitted as one `free` box"
        );
        let after = fs::read(&path).unwrap();
        assert_eq!(
            box_count(&after, b"skip"),
            0,
            "the `skip` box is gone, not re-emitted under its own name"
        );
        assert_eq!(box_count(&after, b"free"), 1, "one `free` box remains");
    }

    #[test]
    fn a_date_and_a_position_are_all_or_nothing() {
        // The file has a date carrier but no location carrier: the request
        // cannot be honoured, so the date it *could* have patched is left
        // alone too.
        let (_dir, path) = temp_copy("plain.mp4", "test-data/test_video_with_date.mp4");
        let before = fs::read(&path).unwrap();
        let edit = VideoMetadataEdit {
            taken_at: Some("2024-07-04T12:00:00Z".parse().unwrap()),
            latitude: Some(52.52),
            longitude: Some(13.405),
        };
        assert!(matches!(
            write_metadata(&path, &edit).unwrap_err(),
            Mp4MetadataError::NoLocationCarrier
        ));
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn a_grown_render_never_moves_the_bytes_behind_the_moov() {
        let (_dir, path) = synthetic_mp4_with_carrier(
            CarrierSpec::MdtaKey(MDTA_LOCATION_KEYS[0]),
            b"+8.2082+016.3737/",
            16,
        );
        // Something that looks like media behind the metadata: only the `moov`
        // region may change, and only within the length it already declares.
        let mut bytes = fs::read(&path).unwrap();
        bytes.extend_from_slice(&box_bytes(b"mdat", b"not really samples"));
        fs::write(&path, &bytes).unwrap();

        let before = fs::read(&path).unwrap();
        let (moov_offset, moov_len) = locate_moov_in(&before).unwrap();
        write_metadata(
            &path,
            &VideoMetadataEdit {
                latitude: Some(52.52),
                longitude: Some(13.405),
                ..Default::default()
            },
        )
        .unwrap();

        let after = fs::read(&path).unwrap();
        assert_eq!(after.len(), before.len());
        assert_eq!(&after[..moov_offset], &before[..moov_offset]);
        assert_eq!(
            &after[moov_offset + moov_len..],
            &before[moov_offset + moov_len..],
            "the media bytes are untouched"
        );
        assert_ne!(
            &after[moov_offset..moov_offset + moov_len],
            &before[moov_offset..moov_offset + moov_len],
            "the grown carrier is inside the moov"
        );
        assert_eq!(
            read_metadata(&path).unwrap().location_iso6709.as_deref(),
            Some("+52.5200+013.4050/")
        );
    }

    #[test]
    fn a_growth_into_the_files_own_room_rebuilds_a_real_moov() {
        // An ffmpeg file whose carrier text is narrower than the position the
        // save wants, with room behind it: the `moov` has to be rebuilt — every
        // box size above the carrier rewritten — without moving a byte of the
        // `mdat` that follows it.
        let (_dir, path) = prepared_growth_fixture("grown.mp4", 16);
        let before = fs::read(&path).unwrap();
        let before_mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let (moov_offset, moov_len) = locate_moov_in(&before).unwrap();
        assert_eq!(
            read_metadata(&path).unwrap().location_iso6709.as_deref(),
            Some("+8.2082+16.3737/"),
            "the prepared carrier is readable"
        );

        let out = write_metadata(
            &path,
            &VideoMetadataEdit {
                latitude: Some(52.52),
                longitude: Some(13.405),
                ..Default::default()
            },
        )
        .unwrap();

        let after = fs::read(&path).unwrap();
        assert_eq!(after.len(), before.len(), "the file length is unchanged");
        assert_eq!(&after[..moov_offset], &before[..moov_offset]);
        assert_eq!(
            &after[moov_offset + moov_len..],
            &before[moov_offset + moov_len..],
            "the media behind the moov is untouched"
        );
        let read_back = read_metadata(&path).unwrap();
        // The prepared carrier writes a two-digit longitude, and the shape is
        // kept: the new position is rendered into it, not into some canonical
        // form — one byte longer than the text it replaces.
        assert_eq!(
            read_back.location_iso6709.as_deref(),
            Some("+52.5200+13.4050/")
        );
        assert_eq!(
            read_back.creation_date_text.as_deref(),
            Some("2024-05-01T10:00:00+0200"),
            "the file's own date carrier is left alone"
        );
        // The 24-byte free box paid for the extra byte of the position.
        assert_eq!(trailing_free_box_size(&after), 23);
        // A rebuilt write is a write like any other: the modification time comes
        // back and the fingerprint describes the file that was left behind.
        assert_eq!(
            fs::metadata(&path).unwrap().modified().unwrap(),
            before_mtime,
            "mtime restored"
        );
        assert_eq!(out.fingerprint.file_size, after.len() as u64);
        assert_eq!(
            out.fingerprint.file_modified,
            truncate_to_seconds(before_mtime).unwrap()
        );
    }

    #[test]
    fn a_rebuild_leaves_every_box_it_did_not_have_to_touch_alone() {
        // What a rebuild has to prove about a grown `moov` is that it is the same
        // file with a longer `moov`: every box it rewrote must be the box it was,
        // and every box that did not have to be rewritten must come out byte for
        // byte as it went in. Comparing the two trees box by box says that;
        // comparing lengths and a few values does not — a rebuild that emitted
        // the sample tables in the wrong order, or dropped one, or resized the
        // wrong container, would leave a file ffprobe still reads but a demuxer
        // cannot use, and every other assertion in the suite would pass.
        let (_dir, path) = prepared_growth_fixture("structure.mp4", 16);
        let before = fs::read(&path).unwrap();
        write_metadata(
            &path,
            &VideoMetadataEdit {
                latitude: Some(52.52),
                longitude: Some(13.405),
                ..Default::default()
            },
        )
        .unwrap();
        let after = fs::read(&path).unwrap();

        /// One box of the tree: where it sits in the file, which path it has
        /// from the root, and the bytes it holds.
        struct Entry<'a> {
            path: String,
            offset: usize,
            size: usize,
            bytes: &'a [u8],
        }

        /// Every box, keyed by its path from the root. Padding is left out: a
        /// rebuild spends it, so its contents are not the file's business. Two
        /// children of one type are told apart by their position, so a
        /// duplicate `ilst` item is compared against the same item and not
        /// against its neighbour.
        fn box_index<'a>(buf: &'a [u8], span: &BoxSpan, path: &str, index: &mut Vec<Entry<'a>>) {
            if is_padding(&span.kind) {
                return;
            }
            let at = format!("{path}/{}", String::from_utf8_lossy(&span.kind));
            index.push(Entry {
                path: at.clone(),
                offset: span.offset,
                size: span.size,
                bytes: &buf[span.offset..span.offset + span.size],
            });
            let mut positions = BTreeMap::new();
            span.children.iter().for_each(|child| {
                let position = positions.entry(child.kind).or_insert(0usize);
                let name = format!("{at}[{position}]");
                *position += 1;
                box_index(buf, child, &name, index);
            });
        }

        fn tree<'a>(buf: &'a [u8]) -> Vec<Entry<'a>> {
            let parsed = parse_box_tree(buf, 0, buf.len(), &[]).unwrap();
            let mut index = Vec::new();
            parsed
                .iter()
                .for_each(|span| box_index(buf, span, "", &mut index));
            index
        }
        let (before_index, after_index) = (tree(&before), tree(&after));
        assert_eq!(
            before_index.len(),
            after_index.len(),
            "a rebuild may not add or drop boxes"
        );

        // Everything a rebuild may touch is the chain of boxes the carrier text
        // sits in: the `moov` grew, so every box from it down to the `data` box
        // holding the text is rewritten — one byte shorter each, apart from the
        // text itself, which is one byte longer. Every other box, the sample
        // tables and the media above all, has to come out as it went in.
        let needle = b"+8.2082+16.3737/";
        let carrier = before
            .windows(needle.len())
            .position(|window| window == needle)
            .unwrap();
        let rewritten = before_index
            .iter()
            .filter(|entry| entry.offset <= carrier && carrier < entry.offset + entry.size)
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        assert!(
            rewritten.len() > 5,
            "the carrier chain has to be part of what is compared: {rewritten:?}"
        );

        for (before_box, after_box) in before_index.iter().zip(&after_index) {
            assert_eq!(
                before_box.path, after_box.path,
                "the same boxes, in the same order"
            );
            if rewritten.contains(&before_box.path) {
                assert_ne!(
                    before_box.bytes, after_box.bytes,
                    "{} was supposed to be rewritten",
                    before_box.path
                );
                continue;
            }
            assert_eq!(
                before_box.bytes, after_box.bytes,
                "{} came out changed",
                before_box.path
            );
        }
    }

    #[test]
    fn the_growth_fixture_moves_its_sample_tables_with_its_media() {
        // The growth fixture is a file whose `moov` no longer fits, so the media
        // behind it moved — and a `stco`/`co64` chunk offset is an absolute file
        // position, not an offset into the `mdat`. If the fixture left the
        // tables alone, every file it produced would name chunks 24 bytes into
        // the wrong packet: ffprobe reads the `moov` it was handed and says
        // nothing about whether the frames behind it are the frames the table
        // claims, so nothing else in the suite would notice.
        const FREE_PAYLOAD: usize = 16;
        let (_dir, path) = prepared_growth_fixture("shifted_tables.mp4", FREE_PAYLOAD);
        let grown = fs::read(&path).unwrap();
        let source = fs::read("test-data/test_video_quicktime_keys.mp4").unwrap();

        let mdat_of = |buf: &[u8]| {
            let tree = parse_box_tree(buf, 0, buf.len(), &[]).unwrap();
            let span = tree.iter().find(|span| span.kind == *b"mdat").unwrap();
            (
                span.offset + header_len(buf, span.offset),
                span.offset + span.size,
            )
        };
        let (source_start, source_end) = mdat_of(&source);
        let (grown_start, grown_end) = mdat_of(&grown);
        assert_eq!(
            grown_start - source_start,
            8 + FREE_PAYLOAD - 2,
            "the moov is the free box behind the carrier longer, less the two \
             bytes the shorter carrier text took"
        );

        let offsets = |buf: &[u8]| {
            chunk_offset_fields(buf)
                .iter()
                .map(|(at, width)| read_field(buf, *at, *width))
                .collect::<Vec<u64>>()
        };
        let (source_offsets, grown_offsets) = (offsets(&source), offsets(&grown));
        assert!(
            !source_offsets.is_empty(),
            "the fixture has to hold a sample table for this to mean anything"
        );
        assert_eq!(
            source_offsets.len(),
            grown_offsets.len(),
            "the shift did not touch the tables"
        );
        for (source_offset, grown_offset) in source_offsets.iter().zip(&grown_offsets) {
            assert!(
                (grown_start..grown_end).contains(&(*grown_offset as usize)),
                "a chunk offset {grown_offset} points outside the mdat {grown_start}..{grown_end}"
            );
            assert!(
                (source_start..source_end).contains(&(*source_offset as usize)),
                "the source offset is not inside the source mdat"
            );
            assert_eq!(
                *grown_offset as usize - grown_start,
                *source_offset as usize - source_start,
                "the chunk keeps its place inside the media it moved with"
            );
        }
    }

    #[test]
    fn a_mixed_date_and_location_edit_lands_both_values() {
        // The location carrier sits in front of the date carrier and grows by
        // one byte, while the date rendering is exactly as long as the text it
        // replaces: a rebuild that applied the date edit at its original offset
        // would land it one byte early.
        let (_dir, path) = synthetic_mp4_with_date_and_location_carriers(16);
        let before = fs::read(&path).unwrap();
        let taken_at: DateTime<Utc> = "2025-01-02T03:04:05Z".parse().unwrap();
        write_metadata(
            &path,
            &VideoMetadataEdit {
                taken_at: Some(taken_at),
                latitude: Some(52.52),
                longitude: Some(13.405),
            },
        )
        .unwrap();

        let after = fs::read(&path).unwrap();
        assert_eq!(after.len(), before.len());
        let read_back = read_metadata(&path).unwrap();
        assert_eq!(
            read_back.location_iso6709.as_deref(),
            Some("+52.5200+013.4050/")
        );
        assert_eq!(
            read_back.creation_date_text.as_deref(),
            Some("2025-01-02T05:04:05+0200")
        );
        assert!(after.windows(24).any(|w| w == b"2025-01-02T05:04:05+0200"));
        assert!(!after.windows(24).any(|w| w == b"2024-05-01T10:00:00+0200"));
        assert_eq!(
            count_creation_times(&after, u64::from(quicktime_seconds(taken_at).unwrap())),
            3
        );
    }

    #[test]
    fn a_location_carrier_with_room_to_spare_reads_back_clean() {
        // The stored text is NUL-padded, and the rendering is shorter than the
        // slot: padding has to survive the reader's strip and the writer's fill
        // alike, or the value would come back with a NUL in it.
        let (_dir, path) = synthetic_mp4_with_carrier(
            CarrierSpec::ItemType(XYZ_BOX),
            b"+8.2082+016.3737/\0\0\0",
            0,
        );
        assert_eq!(
            read_metadata(&path).unwrap().location_iso6709.as_deref(),
            Some("+8.2082+016.3737/")
        );

        // The slot is 20 bytes and the rendering 18: nothing grows, so the
        // file's own free box is left exactly as it was.
        write_metadata(
            &path,
            &VideoMetadataEdit {
                latitude: Some(52.52),
                longitude: Some(13.405),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            read_metadata(&path).unwrap().location_iso6709.as_deref(),
            Some("+52.5200+013.4050/")
        );
        let after = fs::read(&path).unwrap();
        assert!(after.windows(18).any(|w| w == b"+52.5200+013.4050/"));
        assert_eq!(trailing_free_box_size(&after), 8);
    }

    #[test]
    fn renders_every_shape_the_writer_accepts_and_refuses_the_rest() {
        let new: DateTime<Utc> = "2024-07-04T12:00:00.25Z".parse().unwrap();
        for (existing, expected) in [
            ("2024-05-01T10:00:00-0730", "2024-07-04T04:30:00-0730"),
            ("2024-05-01 10:00:00-07:30", "2024-07-04 04:30:00-07:30"),
            (
                "2024-05-01T10:00:00.123456789Z",
                "2024-07-04T12:00:00.250000000Z",
            ),
            ("2024-05-01T10:00:00.5Z", "2024-07-04T12:00:00.2Z"),
            ("2024-05-01T00:00:59Z", "2024-07-04T12:00:00Z"),
            ("2024-05-01T10:00:00", "2024-07-04T12:00:00"),
        ] {
            assert_eq!(
                render_date_in_shape(existing, new).unwrap(),
                expected.as_bytes(),
                "{existing}"
            );
        }
        for refused in [
            "May 1st, 2024",
            "2024-05-01T10:00:00+02",
            "2024-05-01T10:00:00+02000",
            "2024-05-01T10:00",
            "2024-05-01T10:00:00+02:0",
            "2024-13-01",
            "2024-05-01T25:00:00Z",
            "2024-05-01T10:00:00.YZ",
            "2024-05-01T10:00:00.1234567890Z",
            "2024-05-01T10:00:00+2",
            "",
        ] {
            assert_eq!(render_date_in_shape(refused, new), None, "{refused}");
        }
    }

    #[test]
    fn a_refused_date_edit_leaves_the_file_byte_identical() {
        let (_dir, path) = temp_copy("clip.mp4", "test-data/test_video_with_date.mp4");
        let before = fs::read(&path).unwrap();
        let before_mtime = fs::metadata(&path).unwrap().modified().unwrap();

        let too_old = VideoMetadataEdit {
            taken_at: Some("1989-12-31T00:00:00Z".parse().unwrap()),
            ..Default::default()
        };
        assert!(matches!(
            write_metadata(&path, &too_old).unwrap_err(),
            Mp4MetadataError::InvalidDate
        ));
        let too_new = VideoMetadataEdit {
            taken_at: Some("2040-02-06T06:28:16Z".parse().unwrap()),
            ..Default::default()
        };
        assert!(matches!(
            write_metadata(&path, &too_new).unwrap_err(),
            Mp4MetadataError::InvalidDate
        ));

        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(
            fs::metadata(&path).unwrap().modified().unwrap(),
            before_mtime
        );
    }

    #[test]
    fn refuses_a_file_whose_clock_predates_the_unix_epoch() {
        // The fingerprint a save hands back is the file's own clock, and the
        // write hands that clock back afterwards. A clock before 1970 has no
        // such instant — the scanner reports no modification time at all for
        // such a file — so there is nothing honest to report, and an invented
        // wall-clock value would mismatch the row on every later scan,
        // re-extracting a file the save never changed. The save refuses instead,
        // before a byte is touched.
        let (_dir, path) = temp_copy("clip.mp4", "test-data/test_video_with_date.mp4");
        let before = fs::read(&path).unwrap();
        let pre_epoch = SystemTime::UNIX_EPOCH - std::time::Duration::from_secs(86_400);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(pre_epoch)
            .unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().modified().unwrap(),
            pre_epoch,
            "the filesystem has to hold the clock the case is about"
        );

        let edit = VideoMetadataEdit {
            taken_at: Some("2024-07-04T12:00:00Z".parse().unwrap()),
            ..Default::default()
        };
        assert!(matches!(
            write_metadata(&path, &edit).unwrap_err(),
            Mp4MetadataError::Unrepresentable(what) if what == "modification time"
        ));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), pre_epoch);
    }

    #[test]
    fn accepts_the_writable_date_window_including_its_bounds() {
        for stamp in [
            "1990-01-01T00:00:00Z",
            "2024-07-04T12:00:00Z",
            "2040-02-06T06:28:15Z",
        ] {
            let (_dir, path) = temp_copy("window.mp4", "test-data/test_video_with_date.mp4");
            let taken_at: DateTime<Utc> = stamp.parse().unwrap();
            write_metadata(
                &path,
                &VideoMetadataEdit {
                    taken_at: Some(taken_at),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(
                read_metadata(&path).unwrap().creation_time.unwrap(),
                taken_at,
                "{stamp} must round-trip"
            );
        }
    }

    #[test]
    fn quicktime_seconds_maps_the_writable_window() {
        assert_eq!(
            quicktime_seconds("1904-01-01T00:00:00Z".parse().unwrap()),
            Some(0)
        );
        assert_eq!(
            quicktime_seconds("2024-07-04T12:00:00Z".parse().unwrap()),
            Some(3_802_939_200)
        );
        assert_eq!(
            quicktime_seconds("2040-02-06T06:28:15Z".parse().unwrap()),
            Some(u32::MAX)
        );
        assert_eq!(
            quicktime_seconds("2040-02-06T06:28:16Z".parse().unwrap()),
            None
        );
        assert_eq!(
            quicktime_seconds("1903-12-31T23:59:59Z".parse().unwrap()),
            None
        );
    }

    #[test]
    fn out_of_range_or_unpaired_coordinates_are_refused_before_writing() {
        let (_dir, path) = temp_copy("clip.mp4", "test-data/test_video_with_date.mp4");
        let before = fs::read(&path).unwrap();

        for (latitude, longitude) in [
            (Some(91.0), Some(0.0)),
            (Some(-91.0), Some(0.0)),
            (Some(0.0), Some(181.0)),
            (Some(0.0), Some(-181.0)),
            (Some(f64::NAN), Some(0.0)),
            (Some(52.0), None),
            (None, Some(13.0)),
        ] {
            let edit = VideoMetadataEdit {
                taken_at: None,
                latitude,
                longitude,
            };
            assert!(
                matches!(
                    write_metadata(&path, &edit).unwrap_err(),
                    Mp4MetadataError::InvalidCoordinates
                ),
                "latitude {latitude:?} / longitude {longitude:?} must be refused"
            );
        }
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn a_fragmented_movie_is_refused_before_anything_is_written() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("fragmented.mp4");
        let mut moov = box_bytes(b"mvhd", &mvhd_body(0));
        moov.extend_from_slice(&box_bytes(b"mvex", &[]));
        let mut bytes = ftyp_box();
        bytes.extend_from_slice(&box_bytes(b"moov", &moov));
        fs::write(&path, &bytes).unwrap();

        let edit = VideoMetadataEdit {
            taken_at: Some("2024-07-04T12:00:00Z".parse().unwrap()),
            ..Default::default()
        };
        assert!(matches!(
            write_metadata(&path, &edit).unwrap_err(),
            Mp4MetadataError::Fragmented
        ));
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }

    /// A buffer that behaves like a file but refuses the write once its budget
    /// is spent — a full disk, an I/O error, a killed process — so a region
    /// write that fails part-way can be inspected. Writes after the first
    /// failure succeed again, which is what lets the rollback be observed;
    /// [`Self::rollback_fails`] models the device that refuses that one too.
    /// Only `SeekFrom::Start` is supported, which is all the region write seeks
    /// by.
    struct FailingOnceWriter {
        bytes: Vec<u8>,
        position: u64,
        budget: usize,
        failed: bool,
        /// Refuse every write but the first, the way a device that died part-way
        /// through one does: it took the bytes it could, and then it stopped.
        /// Set by the builder before the first write.
        rollback_fails: bool,
        /// How many writes this buffer has been handed, so `rollback_fails` can
        /// spare the first one.
        writes: usize,
        /// The modification time the region write asked to put back, if any.
        restored_modified: Option<SystemTime>,
    }

    impl FailingOnceWriter {
        fn new(bytes: Vec<u8>, budget: usize) -> Self {
            Self {
                bytes,
                position: 0,
                budget,
                failed: false,
                rollback_fails: false,
                writes: 0,
                restored_modified: None,
            }
        }

        /// A device that takes `budget` bytes of the first write and refuses
        /// every write after it: the one that leaves a region half-patched with
        /// nothing to put it back. A budget of 0 would never write a byte at
        /// all, which damages nothing and proves nothing about the report.
        fn dies_partway(bytes: Vec<u8>, budget: usize) -> Self {
            Self {
                rollback_fails: true,
                ..Self::new(bytes, budget)
            }
        }
    }

    impl std::io::Read for FailingOnceWriter {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let start = (self.position as usize).min(self.bytes.len());
            let count = (self.bytes.len() - start).min(out.len());
            out[..count].copy_from_slice(&self.bytes[start..start + count]);
            self.position += count as u64;
            Ok(count)
        }
    }

    impl std::io::Seek for FailingOnceWriter {
        fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
            let std::io::SeekFrom::Start(offset) = position else {
                return Err(std::io::Error::other("unsupported seek"));
            };
            self.position = offset;
            Ok(offset)
        }
    }

    impl std::io::Write for FailingOnceWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.writes += 1;
            if self.rollback_fails && self.writes > 1 {
                return Err(std::io::Error::other("input/output error"));
            }
            let count = if self.failed {
                buf.len()
            } else if self.budget == 0 {
                self.failed = true;
                return Err(std::io::Error::other("no space left on device"));
            } else {
                buf.len().min(self.budget)
            };
            self.budget = self.budget.saturating_sub(count);
            let start = self.position as usize;
            if start + count > self.bytes.len() {
                self.bytes.resize(start + count, 0);
            }
            self.bytes[start..start + count].copy_from_slice(&buf[..count]);
            self.position += count as u64;
            Ok(count)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl RegionTarget for FailingOnceWriter {
        fn restore_modified(&mut self, time: SystemTime) -> std::io::Result<()> {
            self.restored_modified = Some(time);
            Ok(())
        }
    }

    #[test]
    fn a_failed_region_write_puts_the_original_bytes_back() {
        // The write dies after five bytes, the way a full disk or a killed
        // process leaves a region half-patched. What is on the file afterwards
        // has to be the original region, not a mix of old and new bytes
        // (FR-006) — and the file must not look newer than it is either: the
        // change detection and the cache names key on size and modification
        // time, so a save that wrote nothing has to leave the mtime alone.
        let modified = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_234_567);
        let original = b"0000000000000000".to_vec();
        let mut target = FailingOnceWriter::new(original.clone(), 5);

        let err = write_moov_region(
            &mut target,
            0,
            &original,
            b"1111111111111111",
            modified,
            Path::new("clip.mp4"),
        )
        .unwrap_err();
        assert!(matches!(err, Mp4MetadataError::Io(_)), "{err}");
        assert_eq!(target.bytes, original, "the original region is back");
        assert_eq!(
            target.restored_modified,
            Some(modified),
            "the modification time is restored after a failed write"
        );
    }

    #[test]
    fn a_write_the_device_cannot_undo_says_the_file_is_damaged() {
        // The same failure, on a device that takes what it can of the patched
        // bytes and then refuses every write after them — so the rollback
        // cannot run. What is left on the file is a region that is part
        // original and part patched and does not parse, and the client, which
        // was told the save failed, has to learn that from the error rather
        // than from a file that plays as broken weeks later. The modification
        // time stays moved: a write did happen, so the next scan looks at the
        // file again instead of trusting a fingerprint that says nothing
        // changed.
        let original = b"0000000000000000".to_vec();
        let mut target = FailingOnceWriter::dies_partway(original.clone(), 6);

        let err = write_moov_region(
            &mut target,
            0,
            &original,
            b"1111111111111111",
            SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_234_567),
            Path::new("clip.mp4"),
        )
        .unwrap_err();
        assert!(
            matches!(&err, Mp4MetadataError::Io(io) if io.to_string().contains("damaged")),
            "{err}"
        );
        // The damage the error claims has to be real: half the region is the
        // patched bytes and nothing put them back. An assertion that the file
        // still holds the original would pass just as well on a device that
        // never wrote a byte — and say nothing about the report at all.
        assert_eq!(
            target.bytes, b"1111110000000000",
            "the region is half patched, which is what makes it damaged"
        );
        assert_ne!(target.bytes, original);
        assert_eq!(
            target.restored_modified, None,
            "a file whose bytes were never put back must not claim to be unchanged"
        );
    }

    #[test]
    fn a_region_that_no_longer_matches_is_not_overwritten() {
        // The file at the path was replaced between the read that measured the
        // region and the write that patches it: the old `moov` must not land on
        // whatever is there now.
        let original = b"0000000000000000".to_vec();
        let mut target = FailingOnceWriter::new(b"2222222222222222".to_vec(), usize::MAX);

        let err = write_moov_region(
            &mut target,
            0,
            &original,
            b"1111111111111111",
            SystemTime::UNIX_EPOCH,
            Path::new("clip.mp4"),
        )
        .unwrap_err();
        assert!(matches!(err, Mp4MetadataError::NoRoom(_)), "{err}");
        assert_eq!(target.bytes, b"2222222222222222");
    }

    #[test]
    fn a_region_that_no_longer_fits_is_not_overwritten() {
        // The file shrank: the region it was measured in is not there any more.
        let original = b"0000000000000000".to_vec();
        let mut target = FailingOnceWriter::new(b"0000".to_vec(), usize::MAX);

        let err = write_moov_region(
            &mut target,
            0,
            &original,
            b"1111111111111111",
            SystemTime::UNIX_EPOCH,
            Path::new("clip.mp4"),
        )
        .unwrap_err();
        assert!(matches!(err, Mp4MetadataError::NoRoom(_)), "{err}");
        assert_eq!(target.bytes, b"0000");
    }

    #[test]
    fn a_patch_of_another_length_is_refused_before_a_byte_is_written() {
        // The length check is the one refusal that has nothing to do with what
        // the file currently holds: a patched region of a different length would
        // move every byte behind it, so it cannot be written over the region it
        // was rendered for. Both directions are refused, and both leave the file
        // exactly as it was — a longer patch is the dangerous one, because
        // writing it back would push the rest of the `moov` along with it.
        let modified = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_234_567);
        for patched in [
            b"111111111111".to_vec(),       // shorter than the region
            b"111111111111111111".to_vec(), // longer: it would move the bytes behind it
        ] {
            let original = b"0000000000000000".to_vec();
            let mut target = FailingOnceWriter::new(original.clone(), usize::MAX);

            let err = write_moov_region(
                &mut target,
                0,
                &original,
                &patched,
                modified,
                Path::new("clip.mp4"),
            )
            .unwrap_err();
            assert!(
                matches!(err, Mp4MetadataError::NoRoom(_)),
                "a {}-byte patch over a 16-byte region: {err}",
                patched.len()
            );
            assert_eq!(target.bytes, original, "nothing was written");
            assert_eq!(target.writes, 0, "the guard runs before the first write");
            assert_eq!(
                target.restored_modified, None,
                "a refused patch leaves the clock where the file had it"
            );
        }
    }

    #[test]
    fn restores_the_original_moov_after_a_successful_write() {
        let (_dir, path) = temp_copy("clip.mp4", "test-data/test_video_with_date.mp4");
        let before = fs::read(&path).unwrap();
        let before_mtime = fs::metadata(&path).unwrap().modified().unwrap();

        let edit = VideoMetadataEdit {
            taken_at: Some("2024-07-04T12:00:00Z".parse().unwrap()),
            ..Default::default()
        };
        let out = write_metadata(&path, &edit).unwrap();
        assert_ne!(fs::read(&path).unwrap(), before, "the write must land");

        restore(&out.undo).unwrap();
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(
            fs::metadata(&path).unwrap().modified().unwrap(),
            before_mtime
        );
        assert_eq!(
            read_metadata(&path)
                .unwrap()
                .creation_time
                .unwrap()
                .to_rfc3339(),
            "2023-06-15T10:00:00+00:00"
        );
    }

    #[test]
    fn restores_the_original_moov_after_a_rebuilt_grown_write() {
        // A same-length date edit is applied by splicing the region; one that
        // grows a carrier goes through [`rebuild_with_padding`] instead. The
        // undo token describes the region, not the path taken to write it, so a
        // grown save must come back the same way.
        let (_dir, path) = prepared_growth_fixture("grown.mp4", 16);
        let before = fs::read(&path).unwrap();
        let before_mtime = fs::metadata(&path).unwrap().modified().unwrap();

        let out = write_metadata(
            &path,
            &VideoMetadataEdit {
                latitude: Some(52.52),
                longitude: Some(13.405),
                ..Default::default()
            },
        )
        .unwrap();
        let written = fs::read(&path).unwrap();
        assert_ne!(written, before, "the grown write must land");
        assert_eq!(
            trailing_free_box_size(&written),
            23,
            "the rebuild paid for the growth out of the free box"
        );

        restore(&out.undo).unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            before,
            "the whole file is byte-identical to the state before the write"
        );
        assert_eq!(
            fs::metadata(&path).unwrap().modified().unwrap(),
            before_mtime
        );
    }

    #[test]
    fn restore_refuses_a_file_that_changed_length_since_the_write() {
        let (_dir, path) = temp_copy("clip.mp4", "test-data/test_video_with_date.mp4");
        let edit = VideoMetadataEdit {
            taken_at: Some("2024-07-04T12:00:00Z".parse().unwrap()),
            ..Default::default()
        };
        let out = write_metadata(&path, &edit).unwrap();

        // A different file at the same path: the token's region is not this
        // file's any more, so the undo must not be grafted onto it.
        let mut grown = fs::read(&path).unwrap();
        grown.push(0);
        fs::write(&path, &grown).unwrap();
        assert!(matches!(
            restore(&out.undo).unwrap_err(),
            Mp4MetadataError::NoRoom(_)
        ));
        assert_eq!(fs::read(&path).unwrap(), grown);
    }

    #[test]
    fn restore_refuses_a_file_whose_modification_time_moved() {
        let (_dir, path) = temp_copy("clip.mp4", "test-data/test_video_with_date.mp4");
        let edit = VideoMetadataEdit {
            taken_at: Some("2024-07-04T12:00:00Z".parse().unwrap()),
            ..Default::default()
        };
        let out = write_metadata(&path, &edit).unwrap();

        // The same length, so only the modification time says this is not the
        // file the token wrote.
        let touched =
            fs::metadata(&path).unwrap().modified().unwrap() + std::time::Duration::from_secs(30);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(touched)
            .unwrap();
        let before = fs::read(&path).unwrap();
        assert!(matches!(
            restore(&out.undo).unwrap_err(),
            Mp4MetadataError::NoRoom(_)
        ));
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn an_undo_survives_a_filesystem_that_stores_only_whole_seconds() {
        // exFAT, FAT32 and HFS+ keep a modification time with a resolution of
        // two seconds or a whole second, so the stat a write reads back is
        // already rounded — the token's own `modified` comes out of the same
        // rounding, but the file's is re-statted on the way into the undo, and
        // the two do not have to agree to the nanosecond. An undo that demands
        // they do would refuse every edit on those volumes, which is every
        // removable disk and most cameras' cards.
        let (_dir, path) = temp_copy("clip.mp4", "test-data/test_video_with_date.mp4");
        let original = fs::read(&path).unwrap();
        let clock_before_the_save = fs::metadata(&path).unwrap().modified().unwrap();
        let out = write_metadata(&path, &date_edit()).unwrap();
        assert_ne!(
            fs::read(&path).unwrap(),
            original,
            "the fixture's save has to have changed the file"
        );
        let whole_seconds = SystemTime::UNIX_EPOCH
            + std::time::Duration::from_secs(
                out.undo
                    .modified
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            );
        // The clock is re-armed to whole seconds deliberately: the point of the
        // test is that the undo survives a stat the filesystem has already
        // rounded, and on a host that stores nanoseconds the file's own
        // sub-second bytes would make the token's clock and the file's
        // disagree before the undo ever ran. Nothing here may depend on the
        // fixture having kept them.
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(whole_seconds)
            .unwrap();
        // Nothing reads the file between here and the undo: the whole point is
        // that the token, not the content, is what the undo is keyed on.

        restore(&out.undo).unwrap();

        // The undo took: the save is out of the file, and so is the clock the
        // save moved — as far as a whole second can tell. The clock is compared
        // against this copy's own, not the fixture's: a copy is stamped when it
        // is made, and the undo answers for the file it was handed.
        assert_eq!(fs::read(&path).unwrap(), original, "the save is back out");
        assert_eq!(
            truncate_to_seconds(fs::metadata(&path).unwrap().modified().unwrap()),
            truncate_to_seconds(clock_before_the_save),
            "the modification time is back to whole seconds"
        );
    }

    #[test]
    fn an_undo_takes_back_a_save_whatever_form_its_moov_header_has() {
        // A `moov` box states its length three ways: the ordinary 32-bit size,
        // the `largesize` escape for a box over 4 GiB, and — for the last box in
        // a file, as `moov` often is — a size of 0, which means "runs to the end
        // of the file". An undo that recognises its own region by looking at the
        // header reads only the first form, and refuses the other two: the write
        // goes through, the token is armed, and the undo of a save the user just
        // made cannot be taken back. The region is recognised by what the edit
        // would render into it instead, which is true in all three forms.
        let udta = box_bytes(b"udta", &box_bytes(&DAY_BOX, b"2024-05-01T10:00:00+0200"));
        for form in [
            MoovSizeForm::Wide32,
            MoovSizeForm::ToEndOfFile,
            MoovSizeForm::Large64,
        ] {
            let (_dir, path) = synthetic_file_with_moov_size_form(form, &udta);
            let written = fs::read(&path).unwrap();
            let (at, _) = locate_moov_in(&written).unwrap();
            let size_field = u32::from_be_bytes(written[at..at + 4].try_into().unwrap());
            assert_eq!(
                form.size_field(written.len() - at - form.header_len()),
                size_field,
                "{form:?}: the fixture has to use the form it claims to"
            );

            let before = fs::read(&path).unwrap();
            let before_mtime = fs::metadata(&path).unwrap().modified().unwrap();
            let out = write_metadata(&path, &date_edit()).unwrap();
            assert_eq!(
                read_metadata(&path).unwrap().creation_date_text.as_deref(),
                // Rendered into the file's own `©day` shape, offset and all:
                // 12:00 UTC in a +0200 text is 14:00, and a save that wrote "Z"
                // here would be rewriting the shape the fixture chose.
                Some("2024-07-04T14:00:00+0200"),
                "{form:?}: the save lands"
            );

            restore(&out.undo).unwrap();

            // The undo took, and it took back exactly the save: the file is the
            // bytes it was, down to the modification time the write restored.
            assert_eq!(
                fs::read(&path).unwrap(),
                before,
                "{form:?}: the region is back"
            );
            assert_eq!(
                truncate_to_seconds(fs::metadata(&path).unwrap().modified().unwrap()),
                truncate_to_seconds(before_mtime),
                "{form:?}: the clock is back"
            );
        }
    }

    /// An undo token for a region at offset 0 that `edit` patched out of
    /// `restored`, in a file exactly as long as the region.
    ///
    /// The bytes the write put there are rendered by [`patch_moov`] — the call
    /// the write itself made — so a test that is only about the undo does not
    /// have to spell out the patched region, and cannot drift away from what a
    /// save would really have written.
    fn undo_token(restored: &[u8], edit: &VideoMetadataEdit, path: &Path) -> UndoToken {
        let patched = patch_moov(restored, edit).unwrap();
        UndoToken {
            path: path.to_path_buf(),
            moov_offset: 0,
            moov_bytes: restored.to_vec(),
            edit: *edit,
            modified: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_234_567),
            file_len: u64::try_from(patched.len()).unwrap(),
        }
    }

    /// A `moov` region a date edit can genuinely render into: a movie header
    /// holding a creation time, so the patched region differs from the original
    /// in bytes the writer chose rather than in ones the fixture made up.
    fn undoable_region() -> Vec<u8> {
        box_bytes(b"moov", &box_bytes(b"mvhd", &mvhd_body(0)))
    }

    /// A date edit that the fixture above renders differently from its original.
    fn date_edit() -> VideoMetadataEdit {
        VideoMetadataEdit {
            taken_at: Some("2024-07-04T12:00:00Z".parse().unwrap()),
            ..Default::default()
        }
    }

    /// How far a write has to get before it has written the first byte the
    /// original and the patched region differ in.
    ///
    /// One past the first difference, so the write dies inside the patched
    /// bytes. A budget that stops short of it — or that dies inside the box
    /// header the two regions share — leaves the buffer exactly as it was and
    /// proves nothing about a rollback.
    fn budget_into_the_difference(original: &[u8], patched: &[u8]) -> usize {
        let differing = differing_offsets(original, patched);
        assert!(
            !differing.is_empty(),
            "the fixture's save has to change bytes"
        );
        differing[0] + 1
    }

    #[test]
    fn restore_refuses_a_region_that_is_no_longer_the_recorded_moov() {
        // The caller re-arms the modification time before it calls `restore`,
        // because the writer's own `set_modified` may have been refused — so the
        // clock cannot be the guard that says "this is still the file the write
        // produced". Re-arm it here, and a file of the same length and the same
        // clock has nothing left to be refused by but its content.
        let (_dir, path) = temp_copy("clip.mp4", "test-data/test_video_with_date.mp4");
        let out = write_metadata(&path, &date_edit()).unwrap();

        // Something else claimed the region: same length, same clock, no `moov`
        // where the token says one is.
        let mut other = fs::read(&path).unwrap();
        let (at, len) = locate_moov_in(&other).unwrap();
        let mut filler = vec![0u8; len];
        filler[..4].copy_from_slice(&u32::try_from(len).unwrap().to_be_bytes());
        filler[4..8].copy_from_slice(b"free");
        other[at..at + len].copy_from_slice(&filler);
        fs::write(&path, &other).unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(out.undo.modified)
            .unwrap();

        assert!(matches!(
            restore(&out.undo).unwrap_err(),
            Mp4MetadataError::NoRoom(_)
        ));
        assert_eq!(
            fs::read(&path).unwrap(),
            other,
            "the undo is not grafted onto a region that is not the token's"
        );
    }

    #[test]
    fn a_failed_undo_puts_the_patched_region_back() {
        // The same discipline the patch itself is written with: an undo that
        // dies part-way (a full disk, an I/O error) leaves a region that is half
        // original and half patched, and nothing can parse that. What has to be
        // on the file afterwards is the region the undo started from.
        let original = undoable_region();
        let edit = date_edit();
        let patched = patch_moov(&original, &edit).unwrap();
        let mut target = FailingOnceWriter::new(
            patched.clone(),
            budget_into_the_difference(&original, &patched),
        );
        let token = undo_token(&original, &edit, Path::new("clip.mp4"));

        let err = restore_region(&mut target, &token).unwrap_err();
        assert!(matches!(err, Mp4MetadataError::Io(_)), "{err}");
        assert_eq!(target.bytes, patched, "the patched region is back");
        assert_eq!(
            target.restored_modified,
            Some(token.modified),
            "the modification time is restored after a failed undo"
        );
    }

    #[test]
    fn an_undo_nothing_could_put_back_reports_the_damage() {
        // The one outcome the file does not survive intact. Saying so is the
        // whole point: the client is told the edit failed and has to know the
        // file behind it is in an unknown state, because the next scan will
        // re-extract it. And the claim has to be true of the buffer: a device
        // that refused every write would leave nothing to report, so the mock
        // takes the bytes it can first and only then stops accepting them.
        let original = undoable_region();
        let edit = date_edit();
        let patched = patch_moov(&original, &edit).unwrap();
        let mut target = FailingOnceWriter::dies_partway(
            patched.clone(),
            budget_into_the_difference(&original, &patched),
        );
        let token = undo_token(&original, &edit, Path::new("clip.mp4"));

        let Mp4MetadataError::Io(err) = restore_region(&mut target, &token).unwrap_err() else {
            panic!("a failed undo is an I/O error");
        };
        assert!(
            err.to_string().contains("the file is damaged"),
            "the damage is named, not swallowed: {err}"
        );
        assert_ne!(
            target.bytes, patched,
            "the undo wrote something the rollback could not take back"
        );
        assert_ne!(
            target.bytes, original,
            "and what is there is still not the original region either"
        );
    }

    #[cfg(unix)]
    #[test]
    fn read_only_targets_are_refused_without_writing() {
        use std::os::unix::fs::PermissionsExt;

        let (_dir, path) = temp_copy("clip.mp4", "test-data/test_video_with_date.mp4");
        let before = fs::read(&path).unwrap();
        let before_mtime = fs::metadata(&path).unwrap().modified().unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();

        // Root, and anything holding CAP_DAC_OVERRIDE, may write a read-only
        // file regardless, so the permission bit has to be the deciding factor
        // for this test to mean anything.
        if fs::OpenOptions::new().write(true).open(&path).is_ok() {
            eprintln!("Skipping read-only test: this user may write a 0444 file");
            return;
        }

        let edit = VideoMetadataEdit {
            taken_at: Some("2024-07-04T12:00:00Z".parse().unwrap()),
            ..Default::default()
        };
        assert!(matches!(
            write_metadata(&path, &edit).unwrap_err(),
            Mp4MetadataError::ReadOnly(_)
        ));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(
            fs::metadata(&path).unwrap().modified().unwrap(),
            before_mtime
        );
    }

    #[test]
    fn truncated_or_empty_files_are_refused() {
        let dir = TempDir::new().unwrap();
        let edit = VideoMetadataEdit {
            taken_at: Some("2024-07-04T12:00:00Z".parse().unwrap()),
            ..Default::default()
        };

        let empty = dir.path().join("empty.mp4");
        fs::write(&empty, []).unwrap();
        assert!(matches!(
            write_metadata(&empty, &edit).unwrap_err(),
            Mp4MetadataError::UnsupportedContainer(_)
        ));

        let truncated = dir.path().join("truncated.mp4");
        let fixture = fs::read("test-data/test_video_with_date.mp4").unwrap();
        fs::write(&truncated, &fixture[..12]).unwrap();
        assert!(matches!(
            write_metadata(&truncated, &edit).unwrap_err(),
            Mp4MetadataError::UnsupportedContainer(_)
        ));

        let missing = dir.path().join("missing.mp4");
        assert!(matches!(
            write_metadata(&missing, &edit).unwrap_err(),
            Mp4MetadataError::MissingFile
        ));

        let mkv = dir.path().join("clip.mkv");
        fs::copy("test-data/test_video_long.mkv", &mkv).unwrap();
        let mkv_before = fs::read(&mkv).unwrap();
        assert!(matches!(
            write_metadata(&mkv, &edit).unwrap_err(),
            Mp4MetadataError::UnsupportedContainer(_)
        ));
        assert_eq!(fs::read(&mkv).unwrap(), mkv_before);

        let bare = dir.path().join("no-extension");
        fs::copy("test-data/test_video_with_date.mp4", &bare).unwrap();
        assert!(matches!(
            write_metadata(&bare, &edit).unwrap_err(),
            Mp4MetadataError::UnsupportedContainer(_)
        ));
    }

    /// The ffprobe half of the writer's contract: the patched date is what
    /// other tools read back, for a `moov` first and a `moov` last alike.
    #[test]
    fn ffprobe_reports_the_patched_date_and_moov_at_end_files_work_too() {
        use std::process::Command;

        if !video_tests_enabled() {
            eprintln!("Skipping ffprobe patch test: RUN_VIDEO_TESTS not set");
            return;
        }
        let taken_at: DateTime<Utc> = "2024-07-04T12:00:00Z".parse().unwrap();
        for (name, src) in [
            (
                "patched_ftyp_first.mp4",
                "test-data/test_video_with_date.mp4",
            ),
            ("patched_moov_end.mp4", "test-data/test_video_moov_end.mp4"),
        ] {
            let (_dir, path) = temp_copy(name, src);
            write_metadata(
                &path,
                &VideoMetadataEdit {
                    taken_at: Some(taken_at),
                    ..Default::default()
                },
            )
            .unwrap();

            let output = Command::new("ffprobe")
                .args([
                    "-v",
                    "error",
                    "-show_entries",
                    "format_tags=creation_time:stream_tags=creation_time",
                    "-of",
                    "json",
                ])
                .arg(&path)
                .output()
                .expect("ffprobe must be installed for RUN_VIDEO_TESTS");
            assert!(
                output.status.success(),
                "{name}: ffprobe failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

            let tag = |value: &serde_json::Value| {
                value["tags"]["creation_time"]
                    .as_str()
                    .unwrap_or_else(|| panic!("{name}: no creation_time tag in {value}"))
                    .to_string()
            };
            assert_eq!(
                parse_ffprobe_time(&tag(&report["format"])),
                taken_at,
                "{name}: format tag"
            );
            let streams = report["streams"].as_array().expect("streams");
            assert!(!streams.is_empty(), "{name}: no streams");
            for stream in streams {
                assert_eq!(
                    parse_ffprobe_time(&tag(stream)),
                    taken_at,
                    "{name}: stream tag"
                );
            }

            let status = Command::new("ffprobe")
                .args(["-v", "error", "-i"])
                .arg(&path)
                .status()
                .expect("ffprobe must be installed for RUN_VIDEO_TESTS");
            assert!(status.success(), "{name}: ffprobe could not read the file");
        }
    }

    /// The ffprobe half of the location writer's contract: the position is
    /// written in place under the same mdta key, and other tools read it back.
    #[test]
    fn ffprobe_reports_the_patched_location() {
        use std::process::Command;

        if !video_tests_enabled() {
            eprintln!("Skipping ffprobe location test: RUN_VIDEO_TESTS not set");
            return;
        }
        let (_dir, path) = temp_copy(
            "patched_location.mp4",
            "test-data/test_video_quicktime_keys.mp4",
        );
        write_metadata(
            &path,
            &VideoMetadataEdit {
                latitude: Some(52.52),
                longitude: Some(13.405),
                ..Default::default()
            },
        )
        .unwrap();

        let location_of = |path: &Path, expected: &str| {
            let output = Command::new("ffprobe")
                .args([
                    "-v",
                    "error",
                    "-show_entries",
                    "format_tags=com.apple.quicktime.location.ISO6709",
                    "-of",
                    "json",
                ])
                .arg(path)
                .output()
                .expect("ffprobe must be installed for RUN_VIDEO_TESTS");
            assert!(
                output.status.success(),
                "ffprobe failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(
                report["format"]["tags"]["com.apple.quicktime.location.ISO6709"]
                    .as_str()
                    .expect("the location tag"),
                expected
            );
            let status = Command::new("ffprobe")
                .args(["-v", "error", "-i"])
                .arg(path)
                .status()
                .expect("ffprobe must be installed for RUN_VIDEO_TESTS");
            assert!(status.success(), "ffprobe could not read the file");
        };

        // The in-place rewrite: the position replaces the carrier's bytes and
        // nothing else moves.
        location_of(&path, "+52.5200+013.4050/");

        // The rebuild: a real `moov` grown into the file's own padding, every
        // box size above the carrier rewritten, is still a file ffprobe reads.
        let (_dir2, grown) = prepared_growth_fixture("grown_location.mp4", 16);
        location_of(&grown, "+8.2082+16.3737/");
        write_metadata(
            &grown,
            &VideoMetadataEdit {
                latitude: Some(52.52),
                longitude: Some(13.405),
                ..Default::default()
            },
        )
        .unwrap();
        location_of(&grown, "+52.5200+13.4050/");
    }

    /// The ffprobe half of the `loci` reader: ffmpeg's MP4 muxer stores a
    /// location tag as ISO/3GPP `udta/loci`, which is the carrier the
    /// scan-time `-c copy -movflags +faststart` remux leaves behind — so a file
    /// ffmpeg wrote has to read, take a save and read back like any other.
    #[test]
    fn ffprobe_reports_the_patched_loci_location() {
        use std::process::Command;

        if !video_tests_enabled() {
            eprintln!("Skipping ffprobe loci test: RUN_VIDEO_TESTS not set");
            return;
        }
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("loci.mp4");
        let status = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-i",
                "test-data/test_video_with_date.mp4",
                "-c",
                "copy",
                "-metadata",
                "location=+48.2082+016.3737/",
                "-movflags",
                "+faststart",
                "-y",
            ])
            .arg(&path)
            .status()
            .expect("ffmpeg must be installed for RUN_VIDEO_TESTS");
        assert!(status.success(), "ffmpeg could not remux the fixture");
        let before = fs::read(&path).unwrap();
        if box_count(&before, b"loci") == 0 {
            // Not every ffmpeg spells a location tag as `loci`; the synthetic
            // tests carry that layout.
            eprintln!("Skipping loci test: this ffmpeg writes another carrier");
            return;
        }
        assert_eq!(
            read_metadata(&path).unwrap().location_iso6709.as_deref(),
            Some("+48.2082+016.3737/")
        );

        write_metadata(
            &path,
            &VideoMetadataEdit {
                latitude: Some(48.25),
                longitude: Some(16.5),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            read_metadata(&path).unwrap().location_iso6709.as_deref(),
            Some("+48.2500+016.5000/")
        );
        // The pair is patched in place: not one byte of the media behind the
        // `moov` moves.
        let after = fs::read(&path).unwrap();
        let pair = loci_pair_in(&before);
        let changed = differing_offsets(&before, &after);
        assert!(
            changed
                .iter()
                .all(|offset| (pair..pair + 8).contains(offset)),
            "only the pair changed: {changed:?}"
        );

        // Other tools read the patched pair back out of the file.
        let output = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format_tags=location",
                "-of",
                "json",
            ])
            .arg(&path)
            .output()
            .expect("ffprobe must be installed for RUN_VIDEO_TESTS");
        assert!(output.status.success(), "ffprobe could not read the file");
        let reported = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(
            reported.contains("+48.2500+016.5000/"),
            "ffprobe reads the patched position back: {reported}"
        );
    }

    /// `RUN_VIDEO_TESTS` is the project-wide switch for tests that shell out to
    /// ffmpeg/ffprobe; the byte-level tests never depend on it.
    fn video_tests_enabled() -> bool {
        matches!(
            std::env::var("RUN_VIDEO_TESTS").as_deref(),
            Ok("1") | Ok("true")
        )
    }

    fn parse_ffprobe_time(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    /// A fresh copy of a committed fixture, in a directory that outlives the
    /// call: nothing under `test-data/` is ever written to.
    fn temp_copy(name: &str, src: &str) -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(name);
        fs::copy(src, &path).unwrap();
        (dir, path)
    }

    /// Offset and size of the top-level `moov` box in a whole-file buffer.
    fn locate_moov_in(buf: &[u8]) -> Option<(usize, usize)> {
        let tree = parse_box_tree(buf, 0, buf.len(), &[]).ok()?;
        tree.iter()
            .find(|span| span.kind == *b"moov")
            .map(|span| (span.offset, span.size))
    }

    /// Answers how many `ilst` items a read walked into its index since this
    /// thread last asked, and starts the count again.
    fn take_items_walked() -> usize {
        ITEMS_WALKED.with(|walked| walked.replace(0))
    }

    /// Answers how many `ilst` items a save's carrier resolution was handed
    /// since this thread last asked, and starts the count again.
    ///
    /// Counted at [`IlstIndex::items`], the only way a collector reaches an
    /// item: a resolution that walked the item list itself instead resolves the
    /// same carriers and counts none, which is exactly the regression this is
    /// here to catch.
    fn take_carriers_resolved() -> usize {
        CARRIERS_RESOLVED.with(|resolved| resolved.replace(0))
    }

    /// Creation-time field of every `mvhd`/`tkhd`/`mdhd` in a whole-file
    /// buffer, as `(kind, offset, width)`.
    fn date_fields(buf: &[u8]) -> Vec<([u8; 4], usize, usize)> {
        fn walk(buf: &[u8], span: &BoxSpan, out: &mut Vec<([u8; 4], usize, usize)>) {
            if TIME_BOXES.contains(&span.kind) {
                let body = span.offset + header_len(buf, span.offset);
                let width = if buf[body] == 1 { 8 } else { 4 };
                out.push((span.kind, body + 4, width));
            }
            for child in &span.children {
                walk(buf, child, out);
            }
        }

        let tree = parse_box_tree(buf, 0, buf.len(), &[]).unwrap();
        let mut out = Vec::new();
        for span in &tree {
            walk(buf, span, &mut out);
        }
        out
    }

    fn count_creation_times(buf: &[u8], expected_seconds: u64) -> usize {
        date_fields(buf)
            .into_iter()
            .filter(|(_, offset, width)| read_field(buf, *offset, *width) == expected_seconds)
            .count()
    }

    fn read_field(buf: &[u8], offset: usize, width: usize) -> u64 {
        let field = &buf[offset..offset + width];
        if width == 8 {
            u64::from_be_bytes(field.try_into().unwrap())
        } else {
            u64::from(u32::from_be_bytes(field.try_into().unwrap()))
        }
    }

    /// Every offset at which `after` differs from `before`.
    fn differing_offsets(before: &[u8], after: &[u8]) -> Vec<usize> {
        before
            .iter()
            .zip(after)
            .enumerate()
            .filter(|(_, (old, new))| old != new)
            .map(|(offset, _)| offset)
            .collect()
    }

    /// Runs `check` on a worker thread and fails when it does not answer in ten
    /// seconds, so a wrapped box size shows up as a failed test instead of a
    /// suite that never returns.
    fn assert_terminates_within(check: impl FnOnce() -> bool + Send + 'static) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let _worker = std::thread::spawn(move || {
            let _ = sender.send(check());
        });
        assert!(receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the box walk must terminate"));
    }

    /// A `meta` body with no ISO version/flags word: `hdlr`, a one-entry `keys`
    /// box, and an empty `ilst`.
    fn qt_meta_body_without_version_flags() -> Vec<u8> {
        let keys = keys_box(&["com.apple.quicktime.location.ISO6709"]);
        let ilst = box_bytes(b"ilst", &[]);
        [mdta_hdlr(), keys, ilst].concat()
    }

    /// Offset and declared size of every `data` box in a whole-file buffer, in
    /// file order: the payload slots an equal-length rewrite must leave exactly
    /// as it found them.
    fn data_boxes(buf: &[u8]) -> Vec<(usize, usize)> {
        fn walk(span: &BoxSpan, out: &mut Vec<(usize, usize)>) {
            if span.kind == *b"data" {
                out.push((span.offset, span.size));
            }
            for child in &span.children {
                walk(child, out);
            }
        }

        let tree = parse_box_tree(buf, 0, buf.len(), &[]).unwrap();
        let mut out = Vec::new();
        for span in &tree {
            walk(span, &mut out);
        }
        out
    }

    /// Asserts that the payload slots an equal-length rewrite found are the
    /// ones it left.
    ///
    /// The list is checked for emptiness first, and that is the whole point of
    /// the helper: two empty lists compare equal, so a bare
    /// `data_boxes(after) == data_boxes(before)` would pass on a file whose
    /// carrier is not in a `data` box at all — and say nothing about the file
    /// the case is about.
    fn assert_data_boxes_unchanged(before: &[u8], after: &[u8]) {
        let before = data_boxes(before);
        assert!(
            !before.is_empty(),
            "the fixture has to carry data boxes for this to mean anything"
        );
        assert_eq!(
            data_boxes(after),
            before,
            "payload slots are rewritten, never resized"
        );
    }

    /// Size of the trailing `free`/`skip` child of the file's `moov`, `0` when
    /// the file has none: the padding a rebuilt `moov` leaves behind.
    fn trailing_free_box_size(buf: &[u8]) -> usize {
        let tree = parse_box_tree(buf, 0, buf.len(), &[]).unwrap();
        let Some(moov) = tree.iter().find(|span| span.kind == *b"moov") else {
            return 0;
        };
        moov.children
            .iter()
            .rev()
            .find(|child| is_padding(&child.kind))
            .map_or(0, |child| child.size)
    }

    /// Where an `ilst` item of `kind` starts and how long it is, in a
    /// whole-file buffer: the item's own bytes, header included.
    fn ilst_item_span(buf: &[u8], kind: &[u8; 4]) -> (usize, usize) {
        let tree = parse_box_tree(buf, 0, buf.len(), &[]).unwrap();
        let moov = tree.iter().find(|span| span.kind == *b"moov").unwrap();
        let item = find_path(moov, &[*b"udta", META_BOX, *b"ilst"])
            .unwrap()
            .children
            .iter()
            .find(|child| child.kind == *kind)
            .unwrap();
        (item.offset, item.size)
    }

    /// How many boxes of `kind` occur in a whole-file buffer.
    fn box_count(buf: &[u8], kind: &[u8; 4]) -> usize {
        fn walk(span: &BoxSpan, kind: &[u8; 4]) -> usize {
            usize::from(span.kind == *kind)
                + span
                    .children
                    .iter()
                    .map(|child| walk(child, kind))
                    .sum::<usize>()
        }

        let tree = parse_box_tree(buf, 0, buf.len(), &[]).unwrap();
        tree.iter().map(|span| walk(span, kind)).sum()
    }

    /// The text carrier a synthetic fixture should hold.
    #[derive(Debug, Clone, Copy)]
    enum CarrierSpec {
        /// An mdta pair: a `keys` entry with this name, and an `ilst` item
        /// indexed into it.
        MdtaKey(&'static str),
        /// A bare item of `ilst` with this four-character type (`©day`, `©xyz`).
        ItemType([u8; 4]),
    }

    /// A minimal ISO-BMFF file carrying one text carrier, with `free_payload`
    /// spare bytes in a `free` box behind it.
    ///
    /// The committed fixtures hold the carrier shapes ffmpeg and Apple write;
    /// this is how a test reaches the ones they do not — an unparsable value, a
    /// carrier with room to grow.
    fn synthetic_mp4_with_carrier(
        carrier: CarrierSpec,
        payload: &[u8],
        free_payload: usize,
    ) -> (TempDir, PathBuf) {
        let (keys, item) = match carrier {
            CarrierSpec::MdtaKey(name) => (Some(name), text_item_box(&1u32.to_be_bytes(), payload)),
            CarrierSpec::ItemType(kind) => (None, text_item_box(&kind, payload)),
        };
        let mut meta_children = vec![mdta_hdlr()];
        if let Some(name) = keys {
            meta_children.push(keys_box(&[name]));
        }
        meta_children.push(box_bytes(b"ilst", &item));
        // The ISO FullBox form of `meta`: version/flags, then the children.
        let mut meta_body = vec![0u8, 0, 0, 0];
        meta_body.extend_from_slice(&meta_children.concat());
        synthetic_file_with_udta(
            &box_bytes(b"udta", &box_bytes(b"meta", &meta_body)),
            free_payload,
        )
    }

    /// A minimal file carrying a text carrier of `shape` next to a carrier of
    /// the OTHER kind — a `©xyz` location beside the date, or a `©day` date
    /// beside the location — with `free_payload` spare bytes behind them.
    ///
    /// Two carriers at once, because that is the only way to tell "the writer
    /// left this one alone" from "the writer never saw one": a save that names
    /// only the location still has to walk every date carrier in the file to
    /// decide that none of them is the one it was asked for, and the shape
    /// under test is what it walks.
    fn synthetic_mp4_with_carrier_and_counterpart(
        shape: CarrierSpec,
        payload: &[u8],
        free_payload: usize,
    ) -> (TempDir, PathBuf) {
        let (keys, item) = match shape {
            CarrierSpec::MdtaKey(name) => (Some(name), text_item_box(&1u32.to_be_bytes(), payload)),
            CarrierSpec::ItemType(kind) => (None, text_item_box(&kind, payload)),
        };
        // A date is the counterpart of a location and a location the counterpart
        // of a date, so the save under test always has a carrier of its own to
        // land on.
        let (counterpart, counterpart_text) = match shape {
            CarrierSpec::ItemType(kind) if kind == XYZ_BOX => {
                (DAY_BOX, &b"2024-05-01 10:00:00"[..])
            }
            _ => (XYZ_BOX, &b"+48.2082+016.3737/"[..]),
        };
        let mut meta_children = vec![mdta_hdlr()];
        if let Some(name) = keys {
            meta_children.push(keys_box(&[name]));
        }
        let mut ilst = text_item_box(&counterpart, counterpart_text);
        ilst.extend_from_slice(&item);
        meta_children.push(box_bytes(b"ilst", &ilst));
        // The ISO FullBox form of `meta`: version/flags, then the children.
        let mut meta_body = vec![0u8, 0, 0, 0];
        meta_body.extend_from_slice(&meta_children.concat());
        synthetic_file_with_udta(
            &box_bytes(b"udta", &box_bytes(b"meta", &meta_body)),
            free_payload,
        )
    }

    /// A minimal file whose `udta` carries a location in both legacy shapes at
    /// once: a `©xyz` `ilst` item written with four decimals and a solidus, and
    /// a direct `udta/©xyz` child written with five decimals and none — so a
    /// save has to keep two shapes apart, and only the direct child grows.
    /// `free_payload` spare bytes sit behind them.
    fn synthetic_mp4_with_legacy_location_carriers(free_payload: usize) -> (TempDir, PathBuf) {
        let mut meta_body = vec![0u8, 0, 0, 0]; // version + flags
        meta_body.extend_from_slice(&mdta_hdlr());
        meta_body.extend_from_slice(&box_bytes(
            b"ilst",
            &text_item_box(&XYZ_BOX, b"+48.2082+016.3737/"),
        ));
        let mut udta = box_bytes(b"meta", &meta_body);
        udta.extend_from_slice(&box_bytes(&XYZ_BOX, b"+3.86888+151.20930"));
        synthetic_file_with_udta(&box_bytes(b"udta", &udta), free_payload)
    }

    /// A minimal file whose location carrier sits in front of its `©day`
    /// carrier in the same `ilst`, with `free_payload` spare bytes behind them.
    fn synthetic_mp4_with_date_and_location_carriers(free_payload: usize) -> (TempDir, PathBuf) {
        let mut meta_body = vec![0u8, 0, 0, 0]; // version + flags
        meta_body.extend_from_slice(&mdta_hdlr());
        meta_body.extend_from_slice(&keys_box(&[MDTA_LOCATION_KEYS[0]]));
        let mut ilst = text_item_box(&1u32.to_be_bytes(), b"+8.2082+016.3737/");
        ilst.extend_from_slice(&text_item_box(&DAY_BOX, b"2024-05-01T10:00:00+0200"));
        meta_body.extend_from_slice(&box_bytes(b"ilst", &ilst));
        synthetic_file_with_udta(
            &box_bytes(b"udta", &box_bytes(b"meta", &meta_body)),
            free_payload,
        )
    }

    /// A copy of the keys fixture whose location carrier is ready for a save
    /// that has to grow: its text is rewritten with a one-digit latitude and a
    /// two-digit longitude, so a new position needs more bytes than the text
    /// has; the box chain above the carrier is shrunk to match and a `free` box
    /// of `free_payload` bytes is appended as the last child of the `moov`.
    ///
    /// No committed fixture carries padding inside its `moov`, and a save may
    /// only grow into padding, so this is the only way to run the room model
    /// over a box tree as real as ffmpeg writes it — `stbl`/`stco`/`stsz`
    /// tables, a second `trak` and a 58 KB `mdat` behind the metadata.
    ///
    /// That padding is what makes the fixture a grown `moov`, and a grown `moov`
    /// moves the media behind it: the `mdat` starts `8 + free_payload` bytes
    /// higher. Every `stco`/`co64` chunk offset is an absolute position in the
    /// file, so each one is moved by the same delta. A fixture that skipped this
    /// would be a file no demuxer can read — chunk offsets pointing 24 bytes
    /// into the wrong packet — and a rebuild that damaged the sample tables
    /// would still pass every test built on it, because nothing in them looks
    /// at the tables.
    fn prepared_growth_fixture(name: &str, free_payload: usize) -> (TempDir, PathBuf) {
        /// The narrower text the carrier is rewritten with: a save to
        /// 52.52 / 13.405 renders "+52.5200+13.4050/" (17 bytes) into it.
        const TEXT: &[u8] = b"+8.2082+16.3737/";
        const OLD: &[u8] = b"+48.2082+016.3737/";

        let (dir, path) = temp_copy(name, "test-data/test_video_quicktime_keys.mp4");
        let bytes = fs::read(&path).unwrap();
        let tree = parse_box_tree(&bytes, 0, bytes.len(), &[]).unwrap();
        let moov = tree.iter().find(|span| span.kind == *b"moov").unwrap();

        /// The chain from `span` down to the `data` box holding `text`.
        fn chain_to<'a>(
            span: &'a BoxSpan,
            buf: &[u8],
            text: &[u8],
            chain: &mut Vec<&'a BoxSpan>,
        ) -> bool {
            chain.push(span);
            let holds = span.kind == *b"data"
                && buf.get(span.offset + 16..span.offset + span.size) == Some(text);
            if holds
                || span
                    .children
                    .iter()
                    .any(|child| chain_to(child, buf, text, chain))
            {
                return true;
            }
            chain.pop();
            false
        }

        let mut chain = Vec::new();
        assert!(
            chain_to(moov, &bytes, OLD, &mut chain),
            "the fixture stores its location under the mdta key"
        );
        let data = *chain.last().unwrap();
        let shrink = OLD.len() - TEXT.len();
        let mut region = Vec::with_capacity(moov.size + 8 + free_payload);
        region.extend_from_slice(&bytes[moov.offset..data.offset + 16]);
        region.extend_from_slice(TEXT);
        region.extend_from_slice(&bytes[data.offset + data.size..moov.offset + moov.size]);
        // Every box on the chain now holds `shrink` bytes fewer; the fixture
        // writes all of them in the 32-bit size form.
        for span in &chain {
            let at = span.offset - moov.offset;
            region[at..at + 4].copy_from_slice(&((span.size - shrink) as u32).to_be_bytes());
        }
        // The `moov` gives up the two bytes the shorter text takes and gains the
        // `free` box behind the carrier, and the media follows the net of the
        // two: a chunk offset is an absolute file position, so it moves exactly
        // as far as the `mdat` did — no further, and no less.
        let growth = u64::try_from(8 + free_payload).unwrap() - u64::try_from(shrink).unwrap();
        for (at, width) in chunk_offset_fields(&bytes) {
            let at = at - moov.offset;
            let moved = read_field(&region, at, width) + growth;
            assert!(
                moved <= u64::MAX >> (64 - width * 8),
                "the chunk offset no longer fits its field"
            );
            region[at..at + width].copy_from_slice(&moved.to_be_bytes()[8 - width..]);
        }
        region.extend_from_slice(&box_bytes(b"free", &vec![0u8; free_payload]));
        let moov_size = region.len() as u32;
        region[..4].copy_from_slice(&moov_size.to_be_bytes());

        let mut prepared = bytes[..moov.offset].to_vec();
        prepared.extend_from_slice(&region);
        prepared.extend_from_slice(&bytes[moov.offset + moov.size..]);
        fs::write(&path, &prepared).unwrap();
        (dir, path)
    }

    /// Where every chunk offset a whole file's sample tables hold sits: the
    /// field's offset in `buf` and its width — 4 for `stco`, 8 for `co64` — in
    /// file order. The one place the parser can find them, so the growth fixture
    /// and the test that checks its arithmetic cannot drift apart.
    fn chunk_offset_fields(buf: &[u8]) -> Vec<(usize, usize)> {
        fn collect<'a>(buf: &'a [u8], span: &'a BoxSpan, out: &mut Vec<(usize, usize)>) {
            if span.kind == *b"stco" || span.kind == *b"co64" {
                let width = if span.kind == *b"co64" { 8 } else { 4 };
                let body = span.offset + 8;
                let count = read_field(buf, body + 4, 4) as usize;
                (0..count).for_each(|index| out.push((body + 8 + width * index, width)));
            }
            span.children
                .iter()
                .for_each(|child| collect(buf, child, out));
        }
        let tree = parse_box_tree(buf, 0, buf.len(), &[]).unwrap();
        let mut out = Vec::new();
        tree.iter().for_each(|span| collect(buf, span, &mut out));
        out
    }

    /// A minimal file whose `udta` carries its `©day` in all three shapes at
    /// once: an mdta key, an `ilst` item, and a direct `udta` child — each
    /// written differently, so a save has to keep three styles apart.
    fn synthetic_multi_carrier_file() -> (TempDir, PathBuf) {
        let mut meta_body = vec![0u8, 0, 0, 0]; // version + flags
        meta_body.extend_from_slice(&mdta_hdlr());
        meta_body.extend_from_slice(&keys_box(&[MDTA_CREATION_DATE_KEYS[0]]));
        let mut ilst = text_item_box(&1u32.to_be_bytes(), b"2024-05-01T10:00:00+0200");
        ilst.extend_from_slice(&text_item_box(&DAY_BOX, b"2024-05-01 10:00:00"));
        meta_body.extend_from_slice(&box_bytes(b"ilst", &ilst));

        let mut udta_body = box_bytes(b"meta", &meta_body);
        udta_body.extend_from_slice(&box_bytes(&DAY_BOX, b"2024-05-01T10:00:00.500Z"));
        synthetic_file_with_udta(&box_bytes(b"udta", &udta_body), 0)
    }

    /// The file skeleton every synthetic fixture shares: `ftyp`, then a `moov`
    /// holding `mvhd`, one `trak`/`tkhd`/`mdia`/`mdhd`, the given `udta`, and a
    /// `free` box of `free_payload` bytes as the last child of `moov`.
    fn synthetic_file_with_udta(udta: &[u8], free_payload: usize) -> (TempDir, PathBuf) {
        synthetic_file_with_moov(
            &skeleton_moov_body(udta, free_payload),
            MoovSizeForm::Wide32,
        )
    }

    /// The `moov` body every synthetic fixture shares: `mvhd`, one
    /// `trak`/`tkhd`/`mdia`/`mdhd`, the given `udta`, and a `free` box of
    /// `free_payload` bytes as the last child.
    fn skeleton_moov_body(udta: &[u8], free_payload: usize) -> Vec<u8> {
        let mut moov = box_bytes(b"mvhd", &mvhd_body(0));
        let mut trak = box_bytes(b"tkhd", &time_field_body());
        trak.extend_from_slice(&box_bytes(b"mdia", &box_bytes(b"mdhd", &time_field_body())));
        moov.extend_from_slice(&box_bytes(b"trak", &trak));
        moov.extend_from_slice(udta);
        moov.extend_from_slice(&box_bytes(b"free", &vec![0u8; free_payload]));
        moov
    }

    /// A file whose `mvhd`/`tkhd`/`mdhd` are version 1, so their creation fields
    /// are 64 bits wide, storing the given creation and modification times.
    fn synthetic_version_1_time_file(
        udta: &[u8],
        creation: u64,
        modification: u64,
    ) -> (TempDir, PathBuf) {
        let mut moov = box_bytes(b"mvhd", &mvhd_body_v1(creation, modification));
        let mut trak = box_bytes(b"tkhd", &time_field_body_v1(creation, modification));
        trak.extend_from_slice(&box_bytes(
            b"mdia",
            &box_bytes(b"mdhd", &time_field_body_v1(creation, modification)),
        ));
        moov.extend_from_slice(&box_bytes(b"trak", &trak));
        moov.extend_from_slice(udta);
        moov.extend_from_slice(&box_bytes(b"free", &[]));

        synthetic_file_with_moov(&moov, MoovSizeForm::Wide32)
    }

    /// The three ways a box header can state a length.
    ///
    /// Which one a `moov` uses is the writer's business, not the file's: the
    /// same save has to work in all three.
    #[derive(Debug, Clone, Copy)]
    enum MoovSizeForm {
        /// The ordinary 32-bit size, the whole box including its header.
        Wide32,
        /// A size of 0, which ISO/IEC 14496-12 defines as "to the end of the
        /// file" — legal, and the usual form for the last box in a file.
        ToEndOfFile,
        /// The `largesize` escape: a size of 1, with the real 64-bit size
        /// following the type in a header of its own.
        Large64,
    }

    impl MoovSizeForm {
        /// The `moov` box header — size, type and the `largesize` of the 64-bit
        /// form — for a body of `body.len()` bytes.
        fn header(self, body_len: usize) -> Vec<u8> {
            let size = |len: usize| u64::try_from(len).unwrap().to_be_bytes().to_vec();
            let mut header = match self {
                Self::Wide32 => u32::try_from(8 + body_len).unwrap().to_be_bytes().to_vec(),
                Self::ToEndOfFile => 0u32.to_be_bytes().to_vec(),
                Self::Large64 => 1u32.to_be_bytes().to_vec(),
            };
            header.extend_from_slice(b"moov");
            if let Self::Large64 = self {
                header.extend_from_slice(&size(16 + body_len));
            }
            header
        }

        /// The bytes this form's header occupies before the body starts.
        fn header_len(self) -> usize {
            match self {
                Self::Large64 => 16,
                Self::Wide32 | Self::ToEndOfFile => 8,
            }
        }

        /// The value this form puts in the 32-bit size field, so a test can
        /// check its fixture really is the form it claims to be.
        fn size_field(self, body_len: usize) -> u32 {
            match self {
                Self::Wide32 => u32::try_from(8 + body_len).unwrap(),
                Self::ToEndOfFile => 0,
                Self::Large64 => 1,
            }
        }
    }

    /// The file skeleton: `ftyp`, then one `moov` holding `moov_body` and
    /// stating its own length in `form`.
    ///
    /// Nothing follows the `moov`, so the "to the end of the file" form is a
    /// length of exactly the rest of the bytes and stays honest when the save
    /// writes a region of the same length back.
    fn synthetic_file_with_moov(moov_body: &[u8], form: MoovSizeForm) -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("synthetic.mp4");
        let mut bytes = ftyp_box();
        bytes.extend_from_slice(&form.header(moov_body.len()));
        bytes.extend_from_slice(moov_body);
        fs::write(&path, &bytes).unwrap();
        (dir, path)
    }

    /// The shared skeleton written with its `moov` header in `form`.
    fn synthetic_file_with_moov_size_form(form: MoovSizeForm, udta: &[u8]) -> (TempDir, PathBuf) {
        synthetic_file_with_moov(&skeleton_moov_body(udta, 0), form)
    }

    /// A version-0 `tkhd`/`mdhd` body: the version/flags word and the
    /// creation/modification pair, which is all the writer's field patch reads.
    fn time_field_body() -> Vec<u8> {
        let mut body = vec![0u8, 0, 0, 0]; // version + flags
        body.extend_from_slice(&0u32.to_be_bytes()); // creation time
        body.extend_from_slice(&0u32.to_be_bytes()); // modification time
        body
    }

    /// A version-1 `tkhd`/`mdhd` body: the version/flags word and the 64-bit
    /// creation/modification pair.
    fn time_field_body_v1(creation: u64, modification: u64) -> Vec<u8> {
        let mut body = vec![1u8, 0, 0, 0]; // version 1 + flags
        body.extend_from_slice(&creation.to_be_bytes());
        body.extend_from_slice(&modification.to_be_bytes());
        body
    }

    /// A `meta` handler box declaring the `mdta` namespace.
    fn mdta_hdlr() -> Vec<u8> {
        box_bytes(
            b"hdlr",
            &[
                0, 0, 0, 0, // version + flags
                0, 0, 0, 0, // predefined
                b'm', b'd', b't', b'a', // handler type
                0, 0, 0, 0, 0, 0, 0, 0,
            ],
        )
    }

    /// A `keys` box for the given mdta key names. An entry is its own size, the
    /// `mdta` namespace and the name — the name is never NUL-terminated.
    fn keys_box(names: &[&str]) -> Vec<u8> {
        let mut body = vec![0u8, 0, 0, 0]; // version + flags
        body.extend_from_slice(&u32::try_from(names.len()).unwrap().to_be_bytes());
        for name in names {
            body.extend_from_slice(&u32::try_from(8 + name.len()).unwrap().to_be_bytes());
            body.extend_from_slice(b"mdta");
            body.extend_from_slice(name.as_bytes());
        }
        box_bytes(b"keys", &body)
    }

    /// A classic QuickTime text atom: the `u16` byte count of the text and a
    /// `u16` language code (English: `0x55c4`) in front of it. This is the
    /// layout ffmpeg writes for a direct `udta` child.
    fn qt_text_atom(kind: &[u8; 4], text: &[u8]) -> Vec<u8> {
        qt_text_atom_in(kind, text, 0x55c4)
    }

    /// [`qt_text_atom`] with an explicit language code, so a case can tell a
    /// writer that carries the stored code along from one that writes its own.
    fn qt_text_atom_in(kind: &[u8; 4], text: &[u8], language: u16) -> Vec<u8> {
        let mut body = u16::try_from(text.len()).unwrap().to_be_bytes().to_vec();
        body.extend_from_slice(&language.to_be_bytes());
        body.extend_from_slice(text);
        box_bytes(kind, &body)
    }

    /// A classic QuickTime text atom, NUL-terminated when `terminated` is set:
    /// the count then covers the terminator too. That is the shape a save
    /// leaves behind when the rendering is shorter than the slot it fills, so
    /// both layouts are fixtures a real file can be read back in.
    fn qt_text_atom_terminated(kind: &[u8; 4], text: &[u8], terminated: bool) -> Vec<u8> {
        if !terminated {
            return qt_text_atom(kind, text);
        }
        let mut body = u16::try_from(text.len() + 1)
            .unwrap()
            .to_be_bytes()
            .to_vec();
        body.extend_from_slice(&0x55c4u16.to_be_bytes());
        body.extend_from_slice(text);
        body.push(0);
        box_bytes(kind, &body)
    }

    /// An ISO/3GPP `loci` box — the carrier ffmpeg's MP4 muxer writes for a
    /// location tag: a FullBox whose payload is the version/flags word, a 2-byte
    /// language, a NUL-terminated name, a role byte and then the 16.16
    /// fixed-point latitude/longitude pair, longitude first. The pair is given
    /// as its raw `i32`s, so a read test does not encode and decode through the
    /// same helper.
    fn loci_box(name: &[u8], role: u8, latitude: i32, longitude: i32) -> Vec<u8> {
        let mut body = vec![0u8, 0, 0, 0]; // version + flags
        body.extend_from_slice(&0x0409u16.to_be_bytes()); // language: en-US
        body.extend_from_slice(name);
        body.push(0); // the name is NUL-terminated
        body.push(role);
        body.extend_from_slice(&longitude.to_be_bytes());
        body.extend_from_slice(&latitude.to_be_bytes());
        box_bytes(b"loci", &body)
    }

    /// Where the 16.16 horizontal pair of a whole file's `udta/loci` box starts.
    fn loci_pair_in(buf: &[u8]) -> usize {
        let tree = parse_box_tree(buf, 0, buf.len(), &[]).unwrap();
        let moov = tree.iter().find(|span| span.kind == *b"moov").unwrap();
        let loci = find_path(moov, &[*b"udta", *b"loci"]).unwrap();
        loci_pair_offset(buf, loci).unwrap()
    }

    /// One `ilst` item: a `data` box holding `payload` as UTF-8 text behind the
    /// type-indicator/locale word the reader skips.
    fn text_item_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut body = vec![0u8, 0, 0, 1]; // type indicator 1: UTF-8 text
        body.extend_from_slice(&0u32.to_be_bytes()); // locale
        body.extend_from_slice(payload);
        box_bytes(kind, &box_bytes(b"data", &body))
    }

    fn ftyp_box() -> Vec<u8> {
        box_bytes(b"ftyp", &[0u8; 16])
    }

    /// A version-0 `mvhd` body with the given stored creation time.
    fn mvhd_body(creation: u32) -> Vec<u8> {
        let mut body = vec![0u8, 0, 0, 0]; // version + flags
        body.extend_from_slice(&creation.to_be_bytes());
        body.extend_from_slice(&0u32.to_be_bytes()); // modification time
        body.extend_from_slice(&1000u32.to_be_bytes()); // timescale
        body.extend_from_slice(&0u32.to_be_bytes()); // duration
        body
    }

    /// A version-1 `mvhd` body: the version/flags word, the 64-bit
    /// creation/modification pair, then the 32-bit timescale and the 64-bit
    /// duration the version declares.
    fn mvhd_body_v1(creation: u64, modification: u64) -> Vec<u8> {
        let mut body = vec![1u8, 0, 0, 0]; // version 1 + flags
        body.extend_from_slice(&creation.to_be_bytes());
        body.extend_from_slice(&modification.to_be_bytes());
        body.extend_from_slice(&1000u32.to_be_bytes()); // timescale
        body.extend_from_slice(&0u64.to_be_bytes()); // duration
        body
    }

    fn box_bytes(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + body.len());
        out.extend_from_slice(&u32::try_from(8 + body.len()).unwrap().to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }
}
