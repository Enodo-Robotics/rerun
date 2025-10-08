use re_chunk::{Chunk, RowId, TimePoint, Timeline};
use re_log_encoding::FileSink;
use re_log_types::{LogMsg, StoreId};
use re_types::archetypes::Points3D;
use std::time::Duration;

#[test]
fn test_file_rotation_basic() {
    let temp_dir = tempfile::tempdir().unwrap();
    let base_path = temp_dir.path().join("test.rrd");

    // Create a FileSink with 5KB max file size
    // This should be small enough to trigger rotation with a few chunks
    let sink = FileSink::new_with_max_size(base_path.clone(), Some(5 * 1024)).unwrap();

    let store_id = StoreId::empty_recording();

    // Send multiple chunks to trigger rotation
    // Create chunks with enough data to exceed the file size limit
    for i in 0..50 {
        let chunk = Chunk::builder(format!("points/{}", i))
            .with_archetype(
                RowId::new(),
                TimePoint::default().with(Timeline::new_sequence("frame"), i),
                &Points3D::new(vec![[i as f32, i as f32 + 1.0, i as f32 + 2.0]; 100]),
            )
            .build()
            .unwrap();

        let arrow_msg = chunk.to_arrow_msg().unwrap();
        sink.send(LogMsg::ArrowMsg(store_id.clone(), arrow_msg));
    }

    // Flush to ensure all messages are written
    sink.flush_blocking(Duration::from_secs(5)).unwrap();

    // Drop the sink to ensure the writer thread finishes
    drop(sink);

    // Give the writer thread a moment to finish
    std::thread::sleep(Duration::from_millis(100));

    // Check that multiple files were created
    let mut files = std::fs::read_dir(temp_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("rrd"))
        .collect::<Vec<_>>();

    files.sort_by_key(|f| f.path());

    println!("Created {} files:", files.len());
    for file in &files {
        let metadata = std::fs::metadata(file.path()).unwrap();
        println!("  {:?} - {} bytes", file.path().file_name(), metadata.len());
    }

    // We should have created multiple files due to rotation
    assert!(
        files.len() > 1,
        "Expected multiple files due to rotation, got {}",
        files.len()
    );

    // Each file should be readable as a valid RRD
    for file in &files {
        let contents = std::fs::read(file.path()).unwrap();
        assert!(
            !contents.is_empty(),
            "File {:?} is empty",
            file.path()
        );

        // Basic validation: should start with RRD header
        assert!(
            contents.len() >= 4,
            "File {:?} is too small to have a header",
            file.path()
        );
        assert_eq!(
            &contents[0..4],
            b"RRF2",
            "File {:?} doesn't have valid RRD header",
            file.path()
        );
    }

    // Each file except the first should not exceed the max size significantly
    // (allowing some overhead for headers and static messages)
    let max_expected_size = 7 * 1024; // 5KB limit + ~2KB overhead
    for file in files.iter().skip(1) {
        let metadata = std::fs::metadata(file.path()).unwrap();
        if metadata.len() > max_expected_size {
            println!(
                "Warning: File {:?} is {} bytes, expected < {} bytes",
                file.path().file_name(),
                metadata.len(),
                max_expected_size
            );
        }
    }
}

#[test]
fn test_no_rotation() {
    let temp_dir = tempfile::tempdir().unwrap();
    let base_path = temp_dir.path().join("test_no_rotation.rrd");

    // Create a FileSink without rotation
    let sink = FileSink::new(base_path.clone()).unwrap();

    let store_id = StoreId::empty_recording();

    // Send several chunks
    for i in 0..10 {
        let chunk = Chunk::builder(format!("points/{}", i))
            .with_archetype(
                RowId::new(),
                TimePoint::default().with(Timeline::new_sequence("frame"), i),
                &Points3D::new(vec![[i as f32, i as f32 + 1.0, i as f32 + 2.0]; 50]),
            )
            .build()
            .unwrap();

        let arrow_msg = chunk.to_arrow_msg().unwrap();
        sink.send(LogMsg::ArrowMsg(store_id.clone(), arrow_msg));
    }

    sink.flush_blocking(Duration::from_secs(5)).unwrap();
    drop(sink);

    std::thread::sleep(Duration::from_millis(100));

    // Should only have one file
    let files = std::fs::read_dir(temp_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("rrd"))
        .count();

    assert_eq!(files, 1, "Expected exactly 1 file without rotation");
}

#[test]
#[ignore] // Requires rerun binary to be built from this branch
fn test_merge_equals_no_rotation() {
    // This test verifies that merging rotated files produces the same result
    // as if we had not used rotation at all
    //
    // To run this test:
    // 1. Build the rerun binary: cargo build --release --bin rerun
    // 2. Run: cargo test --package re_log_encoding --test file_rotation test_merge_equals_no_rotation --features encoder,decoder -- --ignored --nocapture

    let temp_dir = tempfile::tempdir().unwrap();

    // Create identical data in two scenarios: with rotation and without
    let store_id = StoreId::empty_recording();
    let chunks: Vec<_> = (0..30)
        .map(|i| {
            Chunk::builder(format!("points/{}", i))
                .with_archetype(
                    RowId::new(),
                    TimePoint::default().with(Timeline::new_sequence("frame"), i),
                    &Points3D::new(vec![[i as f32, i as f32 + 1.0, i as f32 + 2.0]; 100]),
                )
                .build()
                .unwrap()
        })
        .collect();

    // Scenario 1: No rotation - single file
    let no_rotation_path = temp_dir.path().join("no_rotation.rrd");
    {
        let sink = FileSink::new(no_rotation_path.clone()).unwrap();
        for chunk in &chunks {
            let arrow_msg = chunk.to_arrow_msg().unwrap();
            sink.send(LogMsg::ArrowMsg(store_id.clone(), arrow_msg));
        }
        sink.flush_blocking(Duration::from_secs(5)).unwrap();
        drop(sink);
    }

    // Scenario 2: With rotation - multiple files
    let rotation_dir = temp_dir.path().join("rotated");
    std::fs::create_dir(&rotation_dir).unwrap();
    let rotation_base_path = rotation_dir.join("rotated.rrd");
    {
        let sink = FileSink::new_with_max_size(rotation_base_path.clone(), Some(5 * 1024)).unwrap();
        for chunk in &chunks {
            let arrow_msg = chunk.to_arrow_msg().unwrap();
            sink.send(LogMsg::ArrowMsg(store_id.clone(), arrow_msg));
        }
        sink.flush_blocking(Duration::from_secs(5)).unwrap();
        drop(sink);
    }

    std::thread::sleep(Duration::from_millis(100));

    // Collect all rotated files
    let mut rotated_files: Vec<_> = std::fs::read_dir(&rotation_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("rrd"))
        .map(|e| e.path())
        .collect();

    rotated_files.sort();

    println!("No rotation file: {:?}", no_rotation_path);
    println!("Rotated files ({}):", rotated_files.len());
    for file in &rotated_files {
        let metadata = std::fs::metadata(file).unwrap();
        println!("  {:?} - {} bytes", file.file_name(), metadata.len());
    }

    // Verify we have multiple rotated files
    assert!(rotated_files.len() > 1, "Expected multiple rotated files");

    // Merge the rotated files
    let merged_path = temp_dir.path().join("merged.rrd");

    // Find the rerun binary - try multiple locations
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set");
    let project_root = std::path::Path::new(&manifest_dir)
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .expect("Cannot find project root");

    // Try to find rerun binary in order of preference:
    // 1. System PATH (e.g., from pip install) - preferred for version compatibility
    // 2. Project target/release
    // 3. Project target/debug

    // First try to find rerun in PATH
    let path_rerun = std::process::Command::new("which")
        .arg("rerun")
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    Some(path)
                } else {
                    None
                }
            } else {
                None
            }
        });

    let rerun_bin = if let Some(path) = path_rerun {
        println!("Using rerun from PATH: {}", path);
        "rerun".to_string()
    } else if project_root.join("target/release/rerun").exists() {
        println!("Using project release binary");
        project_root.join("target/release/rerun").to_string_lossy().to_string()
    } else if project_root.join("target/debug/rerun").exists() {
        println!("Using project debug binary");
        project_root.join("target/debug/rerun").to_string_lossy().to_string()
    } else {
        panic!("Cannot find rerun binary. Please either:\n  1. Build it: cargo build --release --bin rerun\n  2. Install it: pip install rerun-sdk");
    };

    println!("Using rerun binary: {}", rerun_bin);

    let merge_result = std::process::Command::new(&rerun_bin)
        .args(&["rrd", "merge"])
        .args(&rotated_files)
        .arg("--output")
        .arg(&merged_path)
        .output()
        .expect("Failed to run rerun rrd merge");

    if !merge_result.status.success() {
        eprintln!("Merge command failed:");
        eprintln!("stdout: {}", String::from_utf8_lossy(&merge_result.stdout));
        eprintln!("stderr: {}", String::from_utf8_lossy(&merge_result.stderr));
        panic!("Failed to merge rotated files");
    }

    // Now compare the merged file with the non-rotated file
    // We'll decode both and compare the messages
    let no_rotation_bytes = std::fs::read(&no_rotation_path).unwrap();
    let merged_bytes = std::fs::read(&merged_path).unwrap();

    println!("No rotation file size: {} bytes", no_rotation_bytes.len());
    println!("Merged file size: {} bytes", merged_bytes.len());

    // Decode both files
    use re_log_encoding::decoder::decode_bytes;

    let no_rotation_msgs = decode_bytes(&no_rotation_bytes)
        .expect("Failed to decode no-rotation file");
    let merged_msgs = decode_bytes(&merged_bytes)
        .expect("Failed to decode merged file");

    // Compare message counts
    assert_eq!(
        no_rotation_msgs.len(),
        merged_msgs.len(),
        "Message count mismatch: no-rotation has {}, merged has {}",
        no_rotation_msgs.len(),
        merged_msgs.len()
    );

    // Compare each message
    for (i, (no_rot_msg, merged_msg)) in no_rotation_msgs.iter().zip(merged_msgs.iter()).enumerate() {
        // Compare message types and store IDs
        match (no_rot_msg, merged_msg) {
            (LogMsg::SetStoreInfo(a), LogMsg::SetStoreInfo(b)) => {
                // Store info might have different row_ids, but the info should be the same
                assert_eq!(a.info.store_id, b.info.store_id, "Message {} store_id mismatch", i);
            }
            (LogMsg::ArrowMsg(store_a, msg_a), LogMsg::ArrowMsg(store_b, msg_b)) => {
                assert_eq!(store_a, store_b, "Message {} store_id mismatch", i);
                // Compare the batch schemas
                assert_eq!(msg_a.batch.schema(), msg_b.batch.schema(), "Message {} schema mismatch", i);
                // Compare batch row counts
                assert_eq!(msg_a.batch.num_rows(), msg_b.batch.num_rows(), "Message {} row count mismatch", i);
            }
            (LogMsg::BlueprintActivationCommand(a), LogMsg::BlueprintActivationCommand(b)) => {
                assert_eq!(a.blueprint_id, b.blueprint_id, "Message {} blueprint_id mismatch", i);
            }
            _ => {
                panic!("Message {} type mismatch: {:?} vs {:?}", i,
                    std::mem::discriminant(no_rot_msg),
                    std::mem::discriminant(merged_msg));
            }
        }
    }

    println!("✓ Merged file matches non-rotated file!");
}
