use std::path::Path;
use std::process::Command;

use tempfile::TempDir;
use turbo_pix::video_processor::{fix_moov_atom, has_moov_at_start};

/// Helper: create a short test video via ffmpeg (HEVC, 3 seconds, blue frame).
/// Returns the path to the generated video inside the given directory.
fn create_test_video(dir: &Path, filename: &str) -> std::path::PathBuf {
    let output_path = dir.join(filename);
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=320x240:d=3",
            "-c:v",
            "libx265",
            "-crf",
            "28",
            output_path.to_str().unwrap(),
        ])
        .output()
        .expect("ffmpeg must be installed to run this test");
    assert!(
        status.status.success(),
        "ffmpeg failed to create test video: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    output_path
}

/// Helper: verify that MOOV atom appears *after* MDAT in ffprobe trace output,
/// confirming MOOV is at the end of the file.
fn assert_moov_at_end(path: &Path) {
    let output = Command::new("ffprobe")
        .args(["-v", "trace", path.to_str().unwrap()])
        .output()
        .expect("ffprobe must be installed");
    assert!(output.status.success(), "ffprobe failed on test video");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let mdat_pos = stderr.find("type:'mdat'");
    let moov_pos = stderr.find("type:'moov'");

    match (mdat_pos, moov_pos) {
        (Some(mdat), Some(moov)) => {
            assert!(
                mdat < moov,
                "Expected MOOV after MDAT (MOOV-at-end), but mdat_pos={mdat}, moov_pos={moov}"
            );
        }
        _ => panic!(
            "Could not find both mdat and moov atoms in ffprobe trace output for {}",
            path.display()
        ),
    }
}

/// Helper: verify video file is valid by running ffprobe.
fn assert_video_valid(path: &Path) {
    let output = Command::new("ffprobe")
        .args(["-v", "error", path.to_str().unwrap()])
        .output()
        .expect("ffprobe must be installed");
    assert!(
        output.status.success(),
        "ffprobe validation failed for {}: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_fix_moov_atom_moves_moov_to_start() {
    // GIVEN: A video file with MOOV atom at the end (default ffmpeg output without faststart)
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let video_path = create_test_video(tmp_dir.path(), "moov_at_end.mp4");

    // Verify MOOV is actually at the end before we fix it
    assert_moov_at_end(&video_path);

    // Confirm our detection function agrees: MOOV is NOT at start
    let is_at_start = has_moov_at_start(&video_path).expect("has_moov_at_start failed");
    assert!(
        !is_at_start,
        "Expected has_moov_at_start() to return false for MOOV-at-end video"
    );

    // WHEN: We call fix_moov_atom
    fix_moov_atom(&video_path).expect("fix_moov_atom failed");

    // THEN: MOOV should now be at the start
    let is_at_start_after = has_moov_at_start(&video_path).expect("has_moov_at_start failed");
    assert!(
        is_at_start_after,
        "Expected has_moov_at_start() to return true after fix_moov_atom()"
    );

    // AND: The video should still be valid
    assert_video_valid(&video_path);
}

#[test]
fn test_fix_moov_atom_noop_when_already_at_start() {
    // GIVEN: A video file with MOOV already at start (using +faststart)
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");

    // Step 1: Create a raw video
    let raw_path = tmp_dir.path().join("raw.mp4");
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=320x240:d=2",
            "-c:v",
            "libx265",
            "-crf",
            "28",
            raw_path.to_str().unwrap(),
        ])
        .output()
        .expect("ffmpeg failed");
    assert!(status.status.success());

    // Step 2: Remux with +faststart to put MOOV at start
    let faststart_path = tmp_dir.path().join("faststart.mp4");
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-i",
            raw_path.to_str().unwrap(),
            "-c",
            "copy",
            "-movflags",
            "+faststart",
            faststart_path.to_str().unwrap(),
        ])
        .output()
        .expect("ffmpeg failed");
    assert!(status.status.success());

    // Verify MOOV is at start
    let is_at_start = has_moov_at_start(&faststart_path).expect("has_moov_at_start failed");
    assert!(is_at_start, "Expected MOOV at start for +faststart video");

    // Record modification time before fix
    let mtime_before = std::fs::metadata(&faststart_path)
        .expect("metadata")
        .modified()
        .expect("mtime");

    // WHEN: We call fix_moov_atom (should be a no-op)
    fix_moov_atom(&faststart_path).expect("fix_moov_atom failed");

    // THEN: File should be unchanged (mtime preserved since it's a no-op)
    let mtime_after = std::fs::metadata(&faststart_path)
        .expect("metadata")
        .modified()
        .expect("mtime");
    assert_eq!(
        mtime_before, mtime_after,
        "fix_moov_atom should be a no-op when MOOV is already at start"
    );

    // AND: Video should still be valid
    assert_video_valid(&faststart_path);
}

#[test]
fn test_has_moov_at_start_detects_moov_at_end() {
    // GIVEN: A video with MOOV at end
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let video_path = create_test_video(tmp_dir.path(), "detect_moov_end.mp4");

    // WHEN: We check MOOV position
    let result = has_moov_at_start(&video_path).expect("has_moov_at_start failed");

    // THEN: It should report MOOV is NOT at start
    assert!(
        !result,
        "Expected has_moov_at_start() to return false for default ffmpeg output"
    );
}
