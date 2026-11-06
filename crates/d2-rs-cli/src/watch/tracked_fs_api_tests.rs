// SPDX-License-Identifier: MPL-2.0
//! Input-only transport for the original trackedFS.Open test contracts.
use super::*;

#[test]
fn import_opener_tracks_regular_files_and_rejects_fifo_without_waiting() {
    use std::os::unix::fs::OpenOptionsExt;
    let directory = std::env::temp_dir().join(format!(
        "d2rs-tracked-import-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    let directory = fs::canonicalize(directory).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let regular = directory.join("inside.d2");
    fs::write(&regular, b"inside").unwrap();
    let fifo = directory.join("import.d2");
    let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: name is a valid NUL-terminated path to a fresh test FIFO.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let dependencies = Arc::new(Mutex::new(BTreeSet::new()));
    let _guard = track_dependencies(dependencies.clone());
    let mut opener = super::super::filesystem_import_opener();
    let mut file = opener(regular.to_str().unwrap()).unwrap();
    let mut bytes = [0; 6];
    assert_eq!(file.read_bytes(&mut bytes), (6, None));
    assert_eq!(&bytes, b"inside");
    assert!(opener(directory.to_str().unwrap()).is_err());
    assert!(opener(directory.join("missing.d2").to_str().unwrap()).is_err());
    assert_eq!(
        *dependencies.lock().unwrap(),
        BTreeSet::from([regular.clone()])
    );
    let (tx, rx) = mpsc::channel();
    let worker_fifo = fifo.clone();
    let worker_dependencies = dependencies.clone();
    let worker = thread::spawn(move || {
        let _guard = track_dependencies(worker_dependencies);
        let result = super::super::filesystem_import_opener()(worker_fifo.to_str().unwrap());
        tx.send(result.err()).unwrap();
    });
    let result = rx.recv_timeout(Duration::from_secs(2));
    if result.is_err() {
        // Release an accidentally blocking reader before failing the test.
        let _ = fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&fifo);
    }
    worker.join().unwrap();
    assert_eq!(
        result.unwrap().unwrap(),
        format!("localfile: {:?} is not a regular file", fifo)
    );
    assert_eq!(*dependencies.lock().unwrap(), BTreeSet::from([regular]));
}

#[test]
#[ignore = "invoked by the original upstream trackedFS tests"]
fn replay_open() {
    let request: serde_json::Value = serde_json::from_slice(
        &fs::read(std::env::var_os("D2RS_TRACKED_REQUEST").unwrap()).unwrap(),
    )
    .unwrap();
    let policy = match request["policy"].as_str().unwrap() {
        "rooted" => {
            d2_rs_scenebuild::ImageLocalFiles::rooted(Path::new(request["root"].as_str().unwrap()))
                .unwrap()
        }
        "unrestricted" => d2_rs_scenebuild::ImageLocalFiles::Unrestricted,
        "denied" => d2_rs_scenebuild::ImageLocalFiles::default(),
        other => panic!("unknown policy {other}"),
    };
    let dependencies = Arc::new(Mutex::new(BTreeSet::new()));
    let _guard = track_dependencies(dependencies.clone());
    let path = request["path"].as_str().unwrap();
    let result = open_tracked_file(&policy, Path::new(path));
    let (error, denied) = match result {
        Ok(file) => {
            drop(file);
            (String::new(), false)
        }
        Err(error) => (error.to_string(), error.denied),
    };
    // Go's append-only opened slice stays nil until an open succeeds.
    let dependencies = dependencies.lock().unwrap();
    let opened =
        (!dependencies.is_empty()).then(|| dependencies.iter().cloned().collect::<Vec<_>>());
    fs::write(
        std::env::var_os("D2RS_TRACKED_RESPONSE").unwrap(),
        serde_json::to_vec(&json!({"error": error, "denied": denied, "opened": opened})).unwrap(),
    )
    .unwrap();
}
