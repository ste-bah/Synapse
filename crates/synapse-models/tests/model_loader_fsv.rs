use std::{
    cell::Cell,
    fs,
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use synapse_core::error_codes;
use synapse_models::{
    DetectOpts, DetectionFrame, Detector, ModelBackend, ModelDescriptor, ModelError, ModelLoader,
    ModelResult, SessionBuildResult, SessionFactory, SessionHandle, model_download_failed,
    sha256_file,
};
use tempfile::NamedTempFile;

struct FakeSessionFactory {
    result: ModelResult<ModelBackend>,
    calls: Cell<u32>,
}

impl FakeSessionFactory {
    const fn success(backend: ModelBackend) -> Self {
        Self {
            result: Ok(backend),
            calls: Cell::new(0),
        }
    }

    fn backend_unavailable() -> Self {
        Self {
            result: Err(ModelError::BackendUnavailable {
                attempted: vec![ModelBackend::Cuda, ModelBackend::DirectMl],
            }),
            calls: Cell::new(0),
        }
    }
}

impl SessionFactory for FakeSessionFactory {
    fn create_session(
        &self,
        _descriptor: &ModelDescriptor,
        _providers: &[ModelBackend],
    ) -> ModelResult<SessionBuildResult> {
        self.calls.set(self.calls.get().saturating_add(1));
        let selected_backend = match &self.result {
            Ok(backend) => *backend,
            Err(ModelError::BackendUnavailable { attempted }) => {
                return Err(ModelError::BackendUnavailable {
                    attempted: attempted.clone(),
                });
            }
            Err(err) => {
                return Err(ModelError::LoadFailed {
                    path: PathBuf::from("synthetic.onnx"),
                    detail: err.to_string(),
                });
            }
        };
        Ok(SessionBuildResult {
            selected_backend,
            session: SessionHandle::Placeholder,
        })
    }
}

fn temp_model(bytes: &[u8]) -> Result<(NamedTempFile, String), Box<dyn std::error::Error>> {
    let mut file = NamedTempFile::new()?;
    file.write_all(bytes)?;
    file.flush()?;
    file.seek(SeekFrom::Start(0))?;
    let digest = sha256_file(file.path())?;
    Ok((file, digest))
}

fn descriptor(path: &Path, sha256: String) -> ModelDescriptor {
    ModelDescriptor {
        id: "synthetic".to_owned(),
        path: path.to_path_buf(),
        sha256,
        input_shape: vec![1, 3, 640, 640],
        class_map: vec!["button".to_owned()],
    }
}

#[test]
fn sha256_file_reads_past_single_buffer_boundary() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = vec![0x5a; (64 * 1024) + 17];
    let (file, expected) = temp_model(&bytes)?;
    println!(
        "source_of_truth=model_file_hash edge=large_file before=bytes:{}",
        bytes.len()
    );
    let after = sha256_file(file.path())?;
    println!("source_of_truth=model_file_hash edge=large_file after={after}");
    assert_eq!(after, expected);
    Ok(())
}

#[test]
fn hash_mismatch_stops_before_session_creation() -> Result<(), Box<dyn std::error::Error>> {
    let (file, actual) = temp_model(b"model-v1")?;
    let loader = ModelLoader::default();
    let factory = FakeSessionFactory::success(ModelBackend::Cpu);
    let before = descriptor(file.path(), "0".repeat(64));
    println!(
        "source_of_truth=model_loader edge=hash_mismatch before=expected:{} actual:{actual}",
        before.sha256
    );
    let after = loader.load_with_factory(before, &factory);
    println!("source_of_truth=model_loader edge=hash_mismatch after={after:?}");
    assert_eq!(
        after.err().map(|err| err.code()),
        Some(error_codes::MODEL_HASH_MISMATCH)
    );
    assert_eq!(factory.calls.get(), 0);
    Ok(())
}

#[test]
fn verified_file_loads_and_reuses_session_id() -> Result<(), Box<dyn std::error::Error>> {
    let (file, digest) = temp_model(b"verified-model")?;
    let loader = ModelLoader::new(vec![
        ModelBackend::Cuda,
        ModelBackend::DirectMl,
        ModelBackend::Cpu,
    ]);
    let factory = FakeSessionFactory::success(ModelBackend::Cpu);
    let before = descriptor(file.path(), format!("sha256:{digest}"));
    println!(
        "source_of_truth=model_loader edge=verified before=path:{} sha256:{}",
        before.path.display(),
        before.sha256
    );
    let after = loader.load_with_factory(before, &factory)?;
    println!(
        "source_of_truth=model_loader edge=verified after=session_id:{} backend:{:?}",
        after.session_id(),
        after.selected_backend()
    );
    assert_eq!(after.selected_backend(), ModelBackend::Cpu);
    assert_eq!(factory.calls.get(), 1);
    let first = after.infer(
        DetectionFrame {
            frame_seq: 7,
            width: 640,
            height: 640,
        },
        DetectOpts::default(),
    )?;
    let second = after.infer(
        DetectionFrame {
            frame_seq: 8,
            width: 640,
            height: 640,
        },
        DetectOpts::default(),
    )?;
    let loaded_session_id = after.session_id();
    println!(
        "source_of_truth=model_detector edge=session_reuse after=session_id:{loaded_session_id} frames:{},{}",
        first.frame_seq, second.frame_seq
    );
    assert_eq!(loaded_session_id, after.session_id());
    assert!(first.items.is_empty());
    assert!(second.items.is_empty());
    Ok(())
}

#[test]
fn missing_yolov10n_file_is_not_an_error() -> Result<(), Box<dyn std::error::Error>> {
    let tempdir = tempfile::tempdir()?;
    let missing = tempdir.path().join("yolov10n_general.onnx");
    let descriptor = ModelDescriptor {
        id: "yolov10n_general".to_owned(),
        path: missing.clone(),
        sha256: "0".repeat(64),
        input_shape: vec![1, 3, 640, 640],
        class_map: Vec::new(),
    };
    let loader = ModelLoader::default();
    let factory = FakeSessionFactory::success(ModelBackend::Cpu);
    println!(
        "source_of_truth=yolov10n_loader edge=missing before=exists:{} path:{}",
        missing.exists(),
        missing.display()
    );
    let after = loader.load_yolov10n_if_present(descriptor, &factory)?;
    println!("source_of_truth=yolov10n_loader edge=missing after={after:?}");
    assert!(after.is_none());
    assert_eq!(factory.calls.get(), 0);
    Ok(())
}

#[test]
fn backend_unavailable_surfaces_provider_attempts() -> Result<(), Box<dyn std::error::Error>> {
    let (file, digest) = temp_model(b"backend-missing")?;
    let loader = ModelLoader::new(vec![ModelBackend::Cuda, ModelBackend::DirectMl]);
    let factory = FakeSessionFactory::backend_unavailable();
    let before = descriptor(file.path(), digest);
    println!(
        "source_of_truth=model_loader edge=backend_unavailable before=providers:{:?}",
        loader.providers()
    );
    let after = loader.load_with_factory(before, &factory);
    println!("source_of_truth=model_loader edge=backend_unavailable after={after:?}");
    let err = after.err().ok_or("expected backend unavailable")?;
    assert_eq!(err.code(), error_codes::MODEL_BACKEND_UNAVAILABLE);
    assert_eq!(factory.calls.get(), 1);
    Ok(())
}

#[test]
fn canonical_yolov10n_descriptor_uses_local_appdata_shape() {
    let descriptor = ModelDescriptor::yolov10n_general("a".repeat(64), vec!["target".to_owned()]);
    println!(
        "source_of_truth=yolov10n_descriptor edge=canonical after=path:{} input_shape:{:?}",
        descriptor.path.display(),
        descriptor.input_shape
    );
    assert_eq!(descriptor.id, "yolov10n_general");
    assert_eq!(descriptor.input_shape, vec![1, 3, 640, 640]);
    assert!(
        descriptor.path.ends_with(
            ["synapse", "models", "yolov10n_general.onnx"]
                .iter()
                .collect::<PathBuf>()
        )
    );
}

#[test]
fn model_error_codes_have_throw_sites() -> Result<(), Box<dyn std::error::Error>> {
    let source = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"))?;
    for (code, throw_site) in [
        (
            error_codes::MODEL_DOWNLOAD_FAILED,
            "ModelError::DownloadFailed",
        ),
        (error_codes::MODEL_HASH_MISMATCH, "ModelError::HashMismatch"),
        (error_codes::MODEL_LOAD_FAILED, "ModelError::LoadFailed"),
        (
            error_codes::MODEL_BACKEND_UNAVAILABLE,
            "ModelError::BackendUnavailable",
        ),
    ] {
        println!("source_of_truth=model_error_code_audit before=code:{code}");
        let count = source.matches(code).count();
        let throw_count = source.matches(throw_site).count();
        println!(
            "source_of_truth=model_error_code_audit after=code:{code} count:{count} throw_site:{throw_site} throw_count:{throw_count}"
        );
        assert!(count > 0, "missing error-code mapping for {code}");
        assert!(throw_count > 0, "missing concrete throw site for {code}");
    }
    Ok(())
}

#[test]
fn model_download_attempt_fails_closed() {
    let before = "https://models.example.invalid/yolov10n_general.onnx";
    println!("source_of_truth=model_download edge=remote_disabled before=source:{before}");
    let after = model_download_failed(before);
    println!(
        "source_of_truth=model_download edge=remote_disabled after=code:{} detail:{}",
        after.code(),
        after
    );
    assert_eq!(after.code(), error_codes::MODEL_DOWNLOAD_FAILED);
}

#[cfg(not(feature = "ort"))]
#[test]
fn default_loader_without_ort_feature_reports_backend_unavailable()
-> Result<(), Box<dyn std::error::Error>> {
    let (file, digest) = temp_model(b"verified-no-runtime")?;
    let loader = ModelLoader::default();
    let before = descriptor(file.path(), digest);
    println!(
        "source_of_truth=ort_loader edge=no_feature before=providers:{:?}",
        loader.providers()
    );
    let after = loader.load(before);
    println!("source_of_truth=ort_loader edge=no_feature after={after:?}");
    assert_eq!(
        after.err().map(|err| err.code()),
        Some(error_codes::MODEL_BACKEND_UNAVAILABLE)
    );
    Ok(())
}
