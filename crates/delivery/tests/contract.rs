use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

use muriarc_delivery::*;
use muriarc_upgrade::DeploymentProfile;
use sha2::{Digest, Sha256};

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn native_bundle(root: &Path) -> ServerBundleManifest {
    let files = [
        ("bin/muriarc-server", BundleFileRole::Server),
        ("bin/muriarcctl", BundleFileRole::Controller),
        (
            "bin/muriarc-upgrade-executor",
            BundleFileRole::UpgradeExecutor,
        ),
        ("bin/muriarc-verifier", BundleFileRole::Verifier),
        (
            "bin/muriarc-release-fixture",
            BundleFileRole::FixtureProducer,
        ),
        ("ui/index.html", BundleFileRole::UiAsset),
        ("deploy/muriarc.service", BundleFileRole::SystemdService),
        ("deploy/muriarc.sysusers", BundleFileRole::Sysusers),
        ("deploy/muriarc.tmpfiles", BundleFileRole::Tmpfiles),
        ("deploy/delivery.json", BundleFileRole::DeliveryDescriptor),
        (
            "deploy/server.env.example",
            BundleFileRole::EnvironmentExample,
        ),
    ]
    .into_iter()
    .map(|(path, role)| {
        let bytes = format!("synthetic {path}").into_bytes();
        let absolute = root.join(path);
        fs::create_dir_all(absolute.parent().unwrap()).unwrap();
        fs::write(&absolute, &bytes).unwrap();
        BundleFile {
            path: path.to_owned(),
            role,
            size_bytes: u64::try_from(bytes.len()).unwrap(),
            sha256: digest(&bytes),
        }
    })
    .collect();
    let manifest = ServerBundleManifest {
        format_version: SERVER_BUNDLE_FORMAT,
        application_version: "1.0.0".parse().unwrap(),
        profile: DeploymentProfile::NativeSystem,
        files,
    };
    manifest.validate().unwrap();
    fs::write(
        root.join(SERVER_BUNDLE_MANIFEST),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    manifest
}

fn managed_bundle(root: &Path) -> ServerBundleManifest {
    let files = [
        ("bin/muriarcctl", BundleFileRole::Controller),
        (
            "bin/muriarc-upgrade-executor",
            BundleFileRole::UpgradeExecutor,
        ),
        ("bin/muriarc-verifier", BundleFileRole::Verifier),
        ("deploy/compose.yaml", BundleFileRole::ComposeFile),
        ("deploy/descriptor.json", BundleFileRole::ComposeDescriptor),
        ("deploy/.env.example", BundleFileRole::EnvironmentExample),
    ]
    .into_iter()
    .map(|(path, role)| {
        let bytes = format!("synthetic {path}").into_bytes();
        let absolute = root.join(path);
        fs::create_dir_all(absolute.parent().unwrap()).unwrap();
        fs::write(&absolute, &bytes).unwrap();
        BundleFile {
            path: path.to_owned(),
            role,
            size_bytes: u64::try_from(bytes.len()).unwrap(),
            sha256: digest(&bytes),
        }
    })
    .collect();
    let manifest = ServerBundleManifest {
        format_version: SERVER_BUNDLE_FORMAT,
        application_version: "1.0.0".parse().unwrap(),
        profile: DeploymentProfile::ManagedCompose,
        files,
    };
    manifest.validate().unwrap();
    fs::write(
        root.join(SERVER_BUNDLE_MANIFEST),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    manifest
}

#[test]
fn bundle_verifier_rejects_tamper_extra_and_traversal() {
    let root = tempfile::tempdir().unwrap();
    let manifest = native_bundle(root.path());
    let (_, verified) =
        verify_server_bundle(root.path(), Some(&manifest.digest().unwrap())).unwrap();
    assert_eq!(verified.file_count, 11);

    fs::write(root.path().join("extra"), b"not registered").unwrap();
    assert!(verify_server_bundle(root.path(), None).is_err());
    fs::remove_file(root.path().join("extra")).unwrap();
    fs::write(root.path().join("ui/index.html"), b"tampered").unwrap();
    assert!(verify_server_bundle(root.path(), None).is_err());

    let mut unsafe_manifest = manifest;
    unsafe_manifest.files[0].path = "../muriarc-server".to_owned();
    assert!(matches!(
        unsafe_manifest.validate(),
        Err(DeliveryError::UnsafePath(_))
    ));
}

#[cfg(unix)]
#[test]
fn bundle_verifier_rejects_symlink_and_stages_immutable_release() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let manifest = native_bundle(root.path());
    let releases = tempfile::tempdir().unwrap();
    let staged = stage_verified_release(root.path(), &manifest, releases.path()).unwrap();
    assert!(staged.join("bin/muriarc-server").is_file());
    assert!(matches!(
        stage_verified_release(root.path(), &manifest, releases.path()),
        Err(DeliveryError::AlreadyInstalled(_))
    ));

    fs::remove_file(root.path().join("ui/index.html")).unwrap();
    symlink(
        root.path().join("bin/muriarc-server"),
        root.path().join("ui/index.html"),
    )
    .unwrap();
    assert!(verify_server_bundle(root.path(), None).is_err());

    let current = releases.path().join("current");
    activate_release_link(&staged, &current).unwrap();
    assert_eq!(fs::read_link(current).unwrap(), staged);
}

#[test]
fn compose_policy_requires_digest_pin_loopback_and_no_socket() {
    validate_digest_pinned_image(&format!(
        "ghcr.io/jarxunlai/muriarc@sha256:{}",
        "a".repeat(64)
    ))
    .unwrap();
    assert!(validate_digest_pinned_image("ghcr.io/jarxunlai/muriarc:latest").is_err());
    let safe = r#"
services:
  server:
    image: ghcr.io/jarxunlai/muriarc@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
    ports: ["127.0.0.1:8787:8787"]
    read_only: true
    cap_drop: [ALL]
    security_opt: [no-new-privileges:true]
"#;
    validate_compose_policy(safe).unwrap();
    assert!(
        validate_compose_policy(&format!("{safe}\n    volumes: [/var/run/docker.sock:/x]"))
            .is_err()
    );
}

#[test]
fn tracked_delivery_templates_satisfy_typed_policy() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let compose =
        fs::read_to_string(repository.join("deploy/managed-compose/compose.yaml")).unwrap();
    validate_compose_policy(&compose).unwrap();
    let native: DeliveryConfig = serde_json::from_slice(
        &fs::read(repository.join("deploy/native-system/delivery.json")).unwrap(),
    )
    .unwrap();
    native.validate().unwrap();
}

#[test]
fn byo_capabilities_fail_closed_when_candidate_or_restore_is_missing() {
    let mut capabilities = DeliveryCapabilities {
        service_control: true,
        postgres_major: Some(17),
        backup_restore: true,
        isolated_candidate_database: true,
        isolated_candidate_storage: true,
        ddl_executor: true,
        verifier: true,
        bundle_signature_verified: true,
        unavailable_reasons: BTreeSet::new(),
    };
    capabilities.require_upgrade_ready().unwrap();
    capabilities.backup_restore = false;
    assert!(capabilities.require_upgrade_ready().is_err());
}

#[derive(Clone, Default)]
struct FakeRunner {
    commands: Arc<Mutex<Vec<CommandSpec>>>,
}

impl CommandRunner for FakeRunner {
    fn run(&self, command: &CommandSpec) -> Result<CommandOutcome, DeliveryError> {
        self.commands.lock().unwrap().push(command.clone());
        Ok(CommandOutcome {
            success: true,
            exit_code: Some(0),
        })
    }
}

#[test]
fn compose_service_control_uses_host_cli_without_build_or_shell() {
    let root = tempfile::tempdir().unwrap();
    let compose = root.path().join("compose.yaml");
    fs::write(&compose, "services: {}\n").unwrap();
    let config = DeliveryConfig {
        format_version: DELIVERY_CONFIG_FORMAT,
        profile: DeploymentProfile::ManagedCompose,
        paths: DeliveryPaths::managed_compose(root.path()),
        service_user: "muriarc".to_owned(),
        loopback_origin: "http://127.0.0.1:8787".to_owned(),
        environment_file: root.path().join("config/server.env"),
        activation_file: root.path().join("control/active.env"),
        compose_project: Some("muriarc-test".to_owned()),
        compose_file: Some(compose),
    };
    let runner = FakeRunner::default();
    let commands = runner.commands.clone();
    let controller = ServerServiceController::new(config, runner).unwrap();
    controller.stop_for_drain().unwrap();
    controller.start_read_only().unwrap();
    assert!(controller.is_active().unwrap());
    let commands = commands.lock().unwrap();
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[0].program, Path::new("/usr/bin/docker"));
    assert!(commands.iter().all(|command| {
        !command.args.contains(&OsString::from("build"))
            && command
                .args
                .iter()
                .all(|argument| argument != "/var/run/docker.sock")
    }));
}

#[cfg(unix)]
#[test]
fn managed_install_requires_trusted_digest_and_writes_verified_receipt() {
    let bundle = tempfile::tempdir().unwrap();
    let manifest = managed_bundle(bundle.path());
    let install = tempfile::tempdir().unwrap();
    let paths = DeliveryPaths::managed_compose(install.path());
    let config = DeliveryConfig {
        format_version: DELIVERY_CONFIG_FORMAT,
        profile: DeploymentProfile::ManagedCompose,
        compose_project: Some("muriarc-test".to_owned()),
        compose_file: Some(paths.current_release.join("deploy/compose.yaml")),
        paths,
        service_user: "muriarc".to_owned(),
        loopback_origin: "http://127.0.0.1:8787".to_owned(),
        environment_file: install.path().join("config/server.env"),
        activation_file: install.path().join("control/active.env"),
    };
    assert!(install_server_bundle(bundle.path(), "", &config).is_err());
    let (receipt, verified) =
        install_server_bundle(bundle.path(), &manifest.digest().unwrap(), &config).unwrap();
    assert_eq!(receipt.manifest_digest, verified.manifest_digest);
    assert_eq!(load_install_state(&config).unwrap(), Some(receipt.clone()));
    assert_eq!(
        fs::read_link(&config.paths.current_release).unwrap(),
        receipt.release_path
    );
    let (repeated, _) =
        install_server_bundle(bundle.path(), &manifest.digest().unwrap(), &config).unwrap();
    assert_eq!(repeated.release_path, receipt.release_path);
}
