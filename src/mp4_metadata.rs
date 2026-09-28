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
//! - a direct `moov/udta/©day` / `moov/udta/©xyz` child.
//!
//! The parser is hand-rolled over `std` on purpose: a general-purpose demuxer
//! returns values, not the byte spans an in-place rewrite needs, and this
//! project adds no dependency for it. Every failure path is an `Err` — a
//! malformed container is never allowed to panic or to size an allocation.
//!
//! Only `moov` is ever read from the file: `mdat` is located by its header and
//! skipped.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};

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

/// Legacy QuickTime item type carrying an ISO 6709 location.
const XYZ_BOX: [u8; 4] = [0xA9, b'x', b'y', b'z'];

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

/// The metadata a container exposes, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mp4Metadata {
    /// `mvhd` creation time. `None` when `mvhd` is absent or stores 0, which
    /// means "never set" — ffprobe reports no `creation_time` for those.
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
    parse_region(buf, start, end.min(buf.len()), &mut enclosing)
}

/// [`parse_box_tree`] over a reusable `enclosing` chain, so descending into a
/// container costs a push/pop instead of a fresh path vector per box.
fn parse_region(
    buf: &[u8],
    start: usize,
    end: usize,
    enclosing: &mut Vec<[u8; 4]>,
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
            let parsed = parse_region(buf, offset + header, offset + size, enclosing);
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
        creation_time: mvhd_creation_time(&buf, moov)?,
        creation_date_text,
        location_iso6709,
    })
}

/// Offset and size of the top-level `moov` box.
///
/// Only box headers are read — 8 bytes each, 16 when the size field is 1 — so a
/// file whose `mdat` is gigabytes costs a handful of seeks, not a full read.
/// `size == 0` means the box runs to EOF.
fn locate_moov(file: &mut File, file_len: u64) -> Result<(u64, u64), Mp4MetadataError> {
    let mut offset = 0u64;
    let mut found_ftyp = false;
    // Scan on the bytes remaining (`file_len - offset`) rather than on
    // `offset + 8 <= file_len`: an accepted box advances `offset` by at most
    // what is left, so the difference cannot wrap — and the guard is written so
    // that it cannot underflow either.
    while offset <= file_len && file_len - offset >= 8 {
        let found = read_file_box(file, offset, file_len)?;
        if !found_ftyp {
            if found.kind != *b"ftyp" {
                return Err(Mp4MetadataError::UnsupportedContainer(sniffed(&found.kind)));
            }
            found_ftyp = true;
        }
        if found.kind == *b"moov" {
            return Ok((found.offset, found.size));
        }
        offset += found.size;
    }
    if !found_ftyp {
        // Too short to even hold one box header: no container here at all.
        return Err(Mp4MetadataError::UnsupportedContainer(
            "not ISO-BMFF".to_string(),
        ));
    }
    Err(Mp4MetadataError::UnsupportedContainer("moov".to_string()))
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
    let mut names = Vec::new();
    let mut offset = body + 8;
    for _ in 0..entry_count {
        // Each entry is `size` (which covers itself and the namespace), the
        // 4-byte namespace, then the key bytes.
        let Some(entry_size) = size_field(buf, offset) else {
            break;
        };
        let entry_size = entry_size as usize;
        if entry_size < 8 || offset + entry_size > end {
            break;
        }
        names.push(text_of(&buf[offset + 8..offset + entry_size]));
        offset += entry_size;
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

/// First mdta value among `names` (in the order given) that the file carries.
fn mdta_value(items: &[ItemValue], keys: &[String], names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        let index = keys.iter().position(|key| key.as_str() == *name)? as u32 + 1;
        items
            .iter()
            .find(|item| item.kind[0] == 0 && u32::from_be_bytes(item.kind) == index)
            .map(|item| item.text.clone())
    })
}

/// First value stored under the legacy `kind` item type.
fn legacy_item_value(items: &[ItemValue], kind: &[u8; 4]) -> Option<String> {
    items
        .iter()
        .find(|item| item.kind == *kind)
        .map(|item| item.text.clone())
}

/// Value of a direct `moov/udta/<kind>` text box — the pre-`meta` QuickTime
/// home of `©day` and `©xyz`.
fn direct_udta_value(buf: &[u8], moov: &BoxSpan, kind: &[u8; 4]) -> Option<String> {
    let udta = find_child(moov, b"udta")?;
    let direct = find_child(udta, kind)?;
    let start = direct.offset + header_len(buf, direct.offset);
    let end = direct.offset + direct.size;
    (start <= end).then(|| text_of(&buf[start..end]))
}

/// The date text and location text the container carries, each from the first
/// carrier that holds one: mdta keys, then the legacy item type, then a direct
/// `udta` child.
fn text_carriers(buf: &[u8], moov: &BoxSpan) -> (Option<String>, Option<String>) {
    let keys = mdta_keys(buf, moov);
    let items = ilst_items(buf, moov);
    let date = mdta_value(&items, &keys, &MDTA_CREATION_DATE_KEYS)
        .or_else(|| legacy_item_value(&items, &DAY_BOX))
        .or_else(|| direct_udta_value(buf, moov, &DAY_BOX));
    let location = mdta_value(&items, &keys, &MDTA_LOCATION_KEYS)
        .or_else(|| legacy_item_value(&items, &XYZ_BOX))
        .or_else(|| direct_udta_value(buf, moov, &XYZ_BOX));
    (date, location)
}

/// Box text: UTF-8, with a single trailing NUL stripped (ffmpeg writes none,
/// Apple writes one). No other whitespace is touched — the stored value has to
/// survive a rewrite verbatim.
fn text_of(bytes: &[u8]) -> String {
    String::from_utf8_lossy(trim_trailing_nul(bytes)).into_owned()
}

fn trim_trailing_nul(bytes: &[u8]) -> &[u8] {
    if bytes.last() == Some(&0) {
        &bytes[..bytes.len() - 1]
    } else {
        bytes
    }
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
    modified: SystemTime,
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
/// Today the only carriers are the fixed-width creation timestamps above, whose
/// replacement is as long as the field it overwrites. The text carriers of
/// `©day`/`©xyz` and their mdta keys join this list in later steps, which is why
/// the rebuild below is written as a splice rather than as a byte poke.
struct MoovEdit {
    /// Offset of the region in the original `moov`.
    offset: usize,
    /// Length of the region in the original `moov`.
    replaced: usize,
    /// What replaces it.
    content: Vec<u8>,
}

/// Rewrites the creation timestamps of `mvhd`, `tkhd` and `mdhd` in place, and
/// hands back a fingerprint plus an undo token.
///
/// The container's text carriers (`©day`, `©xyz`, the mdta pair) keep their
/// bytes in this step; a later step renders them into the same rebuild.
///
/// All-or-nothing: the date and the coordinates are validated, the `moov` tree
/// is parsed and the whole replacement is rendered and length-checked before
/// the file is opened for writing, and the write itself is a single `write_all`
/// of the `moov` region. No byte outside `moov` is ever touched, and the file
/// is never truncated or grown.
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
    let (moov_offset, moov_size) = locate_moov(&mut source, original_metadata.len())?;
    let original_moov = read_moov(&mut source, moov_offset, moov_size)?;
    drop(source);
    let tree = parse_box_tree(&original_moov, 0, original_moov.len(), &[])?;
    let moov = tree
        .first()
        .filter(|span| span.kind == *b"moov")
        .ok_or_else(|| Mp4MetadataError::UnsupportedContainer("moov".to_string()))?;
    if contains_kind(moov, b"mvex") {
        return Err(Mp4MetadataError::Fragmented);
    }
    let patched = rebuild_moov(&original_moov, moov, edit)?;

    let mut target = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(open_error)?;
    target
        .seek(SeekFrom::Start(moov_offset))
        .map_err(Mp4MetadataError::Io)?;
    target.write_all(&patched).map_err(Mp4MetadataError::Io)?;
    if let Err(err) = target.set_modified(original_modified) {
        log::warn!(
            "could not restore the modification time of {}: {err}",
            path.display()
        );
    }
    drop(target);

    let written = std::fs::metadata(path).map_err(Mp4MetadataError::Io)?;
    Ok(VideoMetadataWrite {
        fingerprint: Fingerprint {
            file_size: written.len(),
            file_modified: truncate_to_seconds(written.modified().map_err(Mp4MetadataError::Io)?),
        },
        undo: UndoToken {
            path: path.to_path_buf(),
            moov_offset,
            moov_bytes: original_moov,
            modified: original_modified,
        },
    })
}

/// Puts the `moov` region and the modification time back as the token recorded
/// them.
///
/// Refuses to write when the region would no longer fit in the file: the token
/// describes one file's layout, and a file that has changed size since is not
/// that file any more.
pub fn restore(token: &UndoToken) -> Result<(), Mp4MetadataError> {
    let mut target = OpenOptions::new()
        .write(true)
        .open(&token.path)
        .map_err(open_error)?;
    let file_len = target.metadata().map_err(Mp4MetadataError::Io)?.len();
    let end = token
        .moov_offset
        .checked_add(token.moov_bytes.len() as u64)
        .ok_or(Mp4MetadataError::NoRoom("moov"))?;
    if end > file_len {
        return Err(Mp4MetadataError::NoRoom("moov"));
    }
    target
        .seek(SeekFrom::Start(token.moov_offset))
        .map_err(Mp4MetadataError::Io)?;
    target
        .write_all(&token.moov_bytes)
        .map_err(Mp4MetadataError::Io)?;
    if let Err(err) = target.set_modified(token.modified) {
        log::warn!(
            "could not restore the modification time of {}: {err}",
            token.path.display()
        );
    }
    Ok(())
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

/// A modification time in the form the scanner stores: whole seconds.
fn truncate_to_seconds(time: SystemTime) -> DateTime<Utc> {
    time.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| DateTime::from_timestamp(duration.as_secs() as i64, 0))
        .unwrap_or_else(Utc::now)
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

/// Renders `edit` into a `moov` of exactly the length `original` has.
fn rebuild_moov(
    original: &[u8],
    moov: &BoxSpan,
    edit: &VideoMetadataEdit,
) -> Result<Vec<u8>, Mp4MetadataError> {
    apply_edits(original, carriers(original, moov, edit)?)
}

/// Every carrier this writer renders, as regions of `original`.
///
/// Only the fixed-width timestamps today; the text carriers come later. All of
/// them are rendered before anything is written, so a carrier that cannot hold
/// the new value fails the write instead of leaving a half-patched file.
fn carriers(
    buf: &[u8],
    moov: &BoxSpan,
    edit: &VideoMetadataEdit,
) -> Result<Vec<MoovEdit>, Mp4MetadataError> {
    let mut edits = Vec::new();
    if let Some(taken_at) = edit.taken_at {
        let seconds = quicktime_seconds(taken_at).ok_or(Mp4MetadataError::InvalidDate)?;
        collect_time_edits(buf, moov, seconds, &mut edits)?;
    }
    Ok(edits)
}

/// Collects the replacement of every `mvhd`/`tkhd`/`mdhd` creation field at or
/// below `span`.
fn collect_time_edits(
    buf: &[u8],
    span: &BoxSpan,
    seconds: u32,
    edits: &mut Vec<MoovEdit>,
) -> Result<(), Mp4MetadataError> {
    if TIME_BOXES.contains(&span.kind) {
        edits.push(time_edit(buf, span, seconds)?);
    }
    for child in &span.children {
        collect_time_edits(buf, child, seconds, edits)?;
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
    })
}

/// Splices `edits` into `original`.
///
/// A rebuild that would change the container's length has no in-place path yet,
/// so it is refused before the file is opened: nothing may be written until the
/// resized-`moov` case is defined.
fn apply_edits(original: &[u8], mut edits: Vec<MoovEdit>) -> Result<Vec<u8>, Mp4MetadataError> {
    edits.sort_by_key(|edit| edit.offset);
    let removed: usize = edits.iter().map(|edit| edit.replaced).sum();
    let added: usize = edits.iter().map(|edit| edit.content.len()).sum();
    if removed != added {
        return Err(Mp4MetadataError::NoRoom("moov"));
    }

    let mut patched = Vec::with_capacity(original.len());
    let mut cursor = 0usize;
    for edit in &edits {
        if edit.offset < cursor || edit.offset + edit.replaced > original.len() {
            return Err(Mp4MetadataError::NoRoom("moov"));
        }
        patched.extend_from_slice(&original[cursor..edit.offset]);
        patched.extend_from_slice(&edit.content);
        cursor = edit.offset + edit.replaced;
    }
    patched.extend_from_slice(&original[cursor..]);
    debug_assert_eq!(patched.len(), original.len());
    Ok(patched)
}

#[cfg(test)]
mod tests {
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
    fn strips_one_trailing_nul_from_box_text() {
        assert_eq!(text_of(b"encoder"), "encoder");
        assert_eq!(text_of(b"encoder\0"), "encoder");
        assert_eq!(text_of(b"encoder\0\0"), "encoder\0");
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
            truncate_to_seconds(before_mtime)
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
        let hdlr = box_bytes(
            b"hdlr",
            &[
                0, 0, 0, 0, // version + flags
                0, 0, 0, 0, // predefined
                b'm', b'd', b't', b'a', // handler type
                0, 0, 0, 0, 0, 0, 0, 0,
            ],
        );
        let key = b"com.apple.quicktime.location.ISO6709";
        let mut keys_body = vec![0u8, 0, 0, 0]; // version + flags
        keys_body.extend_from_slice(&1u32.to_be_bytes()); // entry count
        keys_body.extend_from_slice(&u32::try_from(8 + 4 + key.len()).unwrap().to_be_bytes());
        keys_body.extend_from_slice(b"mdta");
        keys_body.extend_from_slice(key);
        let keys = box_bytes(b"keys", &keys_body);
        let ilst = box_bytes(b"ilst", &[]);
        [hdlr, keys, ilst].concat()
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

    fn box_bytes(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + body.len());
        out.extend_from_slice(&u32::try_from(8 + body.len()).unwrap().to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }
}
