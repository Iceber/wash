use std::collections::HashMap;
#[cfg(target_family = "unix")]
use std::os::unix::prelude::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{anyhow, Result};
#[cfg(target_family = "unix")]
use command_group::AsyncCommandGroup;
use log::warn;
use reqwest::StatusCode;
use tokio::fs::{create_dir_all, metadata, File};
use tokio::process::{Child, Command};
use tokio_stream::StreamExt;
use tokio_util::io::StreamReader;

const WASMCLOUD_GITHUB_RELEASE_URL: &str =
    "https://github.com/wasmCloud/wasmcloud-otp/releases/download";
#[cfg(target_family = "unix")]
pub const WASMCLOUD_HOST_BIN: &str = "wasmcloud_host";
#[cfg(target_family = "windows")]
pub const WASMCLOUD_HOST_BIN: &str = "wasmcloud_host.exe";

// Any version of wasmCloud under 0.63.0 uses Elixir releases and is incompatible
// See https://github.com/wasmCloud/wasmcloud-otp/pull/616 for the move to burrito releases
const MINIMUM_WASMCLOUD_VERSION: &str = "0.63.0";
const DEFAULT_DASHBOARD_PORT: u16 = 4000;

/// A wrapper around the [ensure_wasmcloud_for_os_arch_pair] function that uses the
/// architecture and operating system of the current host machine.
///
/// # Arguments
///
/// * `version` - Specifies the version of the binary to download in the form of `vX.Y.Z`. Must be at least v0.63.0.
/// * `dir` - Where to unpack the wasmCloud host contents into
/// # Examples
///
/// ```no_run
/// # #[tokio::main]
/// # async fn main() {
/// use wash_lib::start::ensure_wasmcloud;
/// let res = ensure_wasmcloud("v0.63.0", "/tmp/wasmcloud/").await;
/// assert!(res.is_ok());
/// assert!(res.unwrap().to_string_lossy() == "/tmp/wasmcloud/v0.63.0/wasmcloud_host".to_string());
/// # }
/// ```
pub async fn ensure_wasmcloud<P>(version: &str, dir: P) -> Result<PathBuf>
where
    P: AsRef<Path>,
{
    ensure_wasmcloud_for_os_arch_pair(std::env::consts::OS, std::env::consts::ARCH, version, dir)
        .await
}

/// Ensures the `wasmcloud_host` application is installed, returning the path to the executable
/// early if it exists or downloading the specified GitHub release version of the wasmCloud host
/// from <https://github.com/wasmCloud/wasmcloud-otp/releases/> and unpacking the contents for a
/// specified OS/ARCH pair to a directory. Returns the path to the executable.
///
/// # Arguments
///
/// * `os` - Specifies the operating system of the binary to download, e.g. `linux`
/// * `arch` - Specifies the architecture of the binary to download, e.g. `amd64`
/// * `version` - Specifies the version of the binary to download in the form of `vX.Y.Z`. Must be
///   at least v0.63.0.
/// * `dir` - Where to unpack the wasmCloud host contents into. This should be the root level
///   directory where to store hosts. Each host will be stored in a directory maching its version
///   (e.g. "/tmp/wasmcloud/v0.63.0")
/// # Examples
///
/// ```no_run
/// # #[tokio::main]
/// # async fn main() {
/// use wash_lib::start::ensure_wasmcloud_for_os_arch_pair;
/// let os = std::env::consts::OS;
/// let arch = std::env::consts::ARCH;
/// let res = ensure_wasmcloud_for_os_arch_pair(os, arch, "v0.63.0", "/tmp/wasmcloud/").await;
/// assert!(res.is_ok());
/// assert!(res.unwrap().to_string_lossy() == "/tmp/wasmcloud/v0.63.0/wasmcloud_host".to_string());
/// # }
/// ```
pub async fn ensure_wasmcloud_for_os_arch_pair<P>(
    os: &str,
    arch: &str,
    version: &str,
    dir: P,
) -> Result<PathBuf>
where
    P: AsRef<Path>,
{
    check_version(version)?;
    if let Some(dir) = find_wasmcloud_binary(&dir, version).await {
        // wasmCloud already exists, return early
        return Ok(dir);
    }
    // Download wasmCloud host tarball
    download_wasmcloud_for_os_arch_pair(os, arch, version, dir).await
}

/// A wrapper around the [download_wasmcloud_for_os_arch_pair] function that uses the
/// architecture and operating system of the current host machine.
///
/// # Arguments
///
/// * `version` - Specifies the version of the binary to download in the form of `vX.Y.Z`
/// * `dir` - Where to unpack the wasmCloud host contents into. This should be the root level
///   directory where to store hosts. Each host will be stored in a directory maching its version
/// # Examples
///
/// ```no_run
/// # #[tokio::main]
/// # async fn main() {
/// use wash_lib::start::download_wasmcloud;
/// let res = download_wasmcloud("v0.57.1", "/tmp/wasmcloud/").await;
/// assert!(res.is_ok());
/// assert!(res.unwrap().to_string_lossy() == "/tmp/wasmcloud/v0.63.0/wasmcloud_host".to_string());
/// # }
/// ```
pub async fn download_wasmcloud<P>(version: &str, dir: P) -> Result<PathBuf>
where
    P: AsRef<Path>,
{
    download_wasmcloud_for_os_arch_pair(std::env::consts::OS, std::env::consts::ARCH, version, dir)
        .await
}

/// Downloads the specified GitHub release version of the wasmCloud host from
/// <https://github.com/wasmCloud/wasmcloud-otp/releases/> and unpacking the contents for a
/// specified OS/ARCH pair to a directory. Returns the path to the Elixir executable.
///
/// # Arguments
///
/// * `os` - Specifies the operating system of the binary to download, e.g. `linux`
/// * `arch` - Specifies the architecture of the binary to download, e.g. `amd64`
/// * `version` - Specifies the version of the binary to download in the form of `vX.Y.Z`
/// * `dir` - Where to unpack the wasmCloud host contents into. This should be the root level
///   directory where to store hosts. Each host will be stored in a directory maching its version
/// # Examples
///
/// ```no_run
/// # #[tokio::main]
/// # async fn main() {
/// use wash_lib::start::download_wasmcloud_for_os_arch_pair;
/// let os = std::env::consts::OS;
/// let arch = std::env::consts::ARCH;
/// let res = download_wasmcloud_for_os_arch_pair(os, arch, "v0.63.0", "/tmp/wasmcloud/").await;
/// assert!(res.is_ok());
/// assert!(res.unwrap().to_string_lossy() == "/tmp/wasmcloud/v0.63.0/wasmcloud_host".to_string());
/// # }
/// ```
pub async fn download_wasmcloud_for_os_arch_pair<P>(
    os: &str,
    arch: &str,
    version: &str,
    dir: P,
) -> Result<PathBuf>
where
    P: AsRef<Path>,
{
    let url = wasmcloud_url(os, arch, version);
    // NOTE(brooksmtownsend): This seems like a lot of work when I really just want to use AsyncRead
    // to pipe the response body into a file. I'm not sure if there's a better way to do this.
    let download_response = reqwest::get(url.clone()).await?;
    if download_response.status() != StatusCode::OK {
        return Err(anyhow!(
            "Failed to download wasmCloud host from {}. Status code: {}",
            url,
            download_response.status()
        ));
    }

    let burrito_bites_stream = download_response
        .bytes_stream()
        .map(|result| result.map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err)));
    let mut wasmcloud_host_burrito = StreamReader::new(burrito_bites_stream);
    let version_dir = dir.as_ref().join(version);
    let file_path = version_dir.join(WASMCLOUD_HOST_BIN);
    if let Some(parent_folder) = file_path.parent() {
        // If the user doesn't have permission to create files in the provided directory,
        // this will bubble the error up noting permission denied
        create_dir_all(parent_folder).await?;
    }
    if let Ok(mut wasmcloud_file) = File::create(&file_path).await {
        // This isn't an `if let` to avoid a Windows lint warning
        if file_path.file_name().is_some() {
            // Set permissions of executable files and binaries to allow executing
            #[cfg(target_family = "unix")]
            {
                let mut perms = wasmcloud_file.metadata().await?.permissions();
                perms.set_mode(0o755);
                wasmcloud_file.set_permissions(perms).await?;
            }
        }
        tokio::io::copy(&mut wasmcloud_host_burrito, &mut wasmcloud_file).await?;
    }

    // Return success if wasmCloud components exist, error otherwise
    match find_wasmcloud_binary(&dir, version).await {
        Some(path) => Ok(path),
        None => Err(anyhow!(
            "wasmCloud was not installed successfully, please see logs"
        )),
    }
}

/// Helper function to start a wasmCloud host given the path to the burrito release application
/// /// # Arguments
///
/// * `bin_path` - Path to the wasmcloud_host burrito application
/// * `stdout` - Specify where wasmCloud stdout logs should be written to. Logs can be written to stdout by the erlang process
/// * `stderr` - Specify where wasmCloud stderr logs should be written to. Logs are written to stderr that are generated by wasmCloud
/// * `env_vars` - Environment variables to pass to the host, see <https://wasmcloud.dev/reference/host-runtime/host_configure/#supported-configuration-variables> for details
pub async fn start_wasmcloud_host<P, T, S>(
    bin_path: P,
    stdout: T,
    stderr: S,
    env_vars: HashMap<String, String>,
) -> Result<Child>
where
    P: AsRef<Path>,
    T: Into<Stdio>,
    S: Into<Stdio>,
{
    // If we can connect to the local port, a wasmCloud host won't be able to listen on that port
    let port = env_vars
        .get("WASMCLOUD_DASHBOARD_PORT")
        .cloned()
        .unwrap_or_else(|| DEFAULT_DASHBOARD_PORT.to_string());
    if tokio::net::TcpStream::connect(format!("localhost:{port}"))
        .await
        .is_ok()
    {
        return Err(anyhow!(
            "Could not start wasmCloud, a process is already listening on localhost:{}",
            port
        ));
    }

    // Constructing this object in one step results in a temporary value that's dropped
    let mut cmd = Command::new(bin_path.as_ref());
    let cmd = cmd
        // wasmCloud host logs are sent to stderr as of https://github.com/wasmCloud/wasmcloud-otp/pull/418
        .stderr(stderr)
        .stdout(stdout)
        .stdin(Stdio::null())
        .envs(&env_vars);

    #[cfg(target_family = "unix")]
    {
        Ok(cmd.group_spawn()?.into_inner())
    }
    #[cfg(target_family = "windows")]
    {
        Ok(cmd.spawn()?)
    }
}

/// Helper function to indicate if the wasmCloud host tarball is successfully
/// installed in a directory. Returns the path to the binary if it exists
pub async fn find_wasmcloud_binary<P>(dir: P, version: &str) -> Option<PathBuf>
where
    P: AsRef<Path>,
{
    let versioned_dir = dir.as_ref().join(version);
    let bin_file = versioned_dir.join(WASMCLOUD_HOST_BIN);

    metadata(&bin_file).await.is_ok().then_some(bin_file)
}

/// Helper function to determine the wasmCloud host release path given an os/arch and version
fn wasmcloud_url(os: &str, arch: &str, version: &str) -> String {
    // NOTE(brooksmtownsend): I'm hardcoding `gnu` here because I'm not sure how to determine
    // that programmatically. This essentially is what we had before (gnu only) but we do have a musl
    // release that we should consider.
    let os = os
        .replace("macos", "darwin")
        .replace("linux", "linux_gnu")
        .replace("windows", "windows.exe");
    format!("{WASMCLOUD_GITHUB_RELEASE_URL}/{version}/wasmcloud_host_{arch}_{os}")
}

/// Helper function to ensure the version of wasmCloud is above the minimum
/// supported version (v0.63.0) that runs burrito releases
fn check_version(version: &str) -> Result<()> {
    let version_req = semver::VersionReq::parse(&format!(">={MINIMUM_WASMCLOUD_VERSION}"))?;
    match semver::Version::parse(version.trim_start_matches('v')) {
        Ok(parsed_version) if !parsed_version.pre.is_empty() => {
            warn!("Using prerelease version {} of wasmCloud", version);
            Ok(())
        }
        Ok(parsed_version) if !version_req.matches(&parsed_version) => Err(anyhow!(
            "wasmCloud version {} is earlier than the minimum supported version of v{}",
            version,
            MINIMUM_WASMCLOUD_VERSION
        )),
        Ok(_ver) => Ok(()),
        Err(_parse_err) => {
            log::warn!(
                "Failed to parse wasmCloud version as a semantic version, download may fail"
            );
            Ok(())
        }
    }
}
#[cfg(test)]
mod test {
    use super::{check_version, ensure_wasmcloud, wasmcloud_url};
    use crate::start::{
        ensure_nats_server, ensure_wasmcloud_for_os_arch_pair, find_wasmcloud_binary,
        is_bin_installed, start_nats_server, start_wasmcloud_host, NatsConfig, NATS_SERVER_BINARY,
    };
    use reqwest::StatusCode;
    use std::{collections::HashMap, env::temp_dir};
    use tokio::fs::{create_dir_all, remove_dir_all};
    const WASMCLOUD_VERSION: &str = "v0.63.0";

    #[tokio::test]
    async fn can_request_supported_wasmcloud_urls() {
        let host_tarballs = vec![
            wasmcloud_url("linux", "aarch64", WASMCLOUD_VERSION),
            wasmcloud_url("linux", "x86_64", WASMCLOUD_VERSION),
            wasmcloud_url("macos", "aarch64", WASMCLOUD_VERSION),
            wasmcloud_url("macos", "x86_64", WASMCLOUD_VERSION),
            wasmcloud_url("windows", "x86_64", WASMCLOUD_VERSION),
        ];
        for tarball_url in host_tarballs {
            assert_eq!(
                reqwest::get(tarball_url).await.unwrap().status(),
                StatusCode::OK
            );
        }
    }

    #[tokio::test]
    async fn can_download_wasmcloud_burrito() {
        let download_dir = temp_dir().join("can_download_wasmcloud_burrito");
        let res =
            ensure_wasmcloud_for_os_arch_pair("macos", "aarch64", WASMCLOUD_VERSION, &download_dir)
                .await
                .expect("Should be able to download tarball");

        // Make sure we can find the binary and that it matches the path we got back from ensure
        assert_eq!(
            find_wasmcloud_binary(&download_dir, WASMCLOUD_VERSION)
                .await
                .expect("Should have found installed wasmcloud"),
            res
        );

        let _ = remove_dir_all(download_dir).await;
    }

    #[tokio::test]
    async fn can_handle_missing_wasmcloud_version() {
        let download_dir = temp_dir().join("can_handle_missing_wasmcloud_version");
        let res = ensure_wasmcloud("v10233.123.3.4", &download_dir).await;

        assert!(res.is_err());
        let _ = remove_dir_all(download_dir).await;
    }

    #[tokio::test]
    async fn can_download_different_versions() {
        let download_dir = temp_dir().join("can_download_different_versions");
        ensure_wasmcloud_for_os_arch_pair("macos", "aarch64", WASMCLOUD_VERSION, &download_dir)
            .await
            .expect("Should be able to download host");

        assert!(
            find_wasmcloud_binary(&download_dir, WASMCLOUD_VERSION)
                .await
                .is_some(),
            "wasmCloud should be installed"
        );

        ensure_wasmcloud_for_os_arch_pair("macos", "aarch64", "v0.63.1", &download_dir)
            .await
            .expect("Should be able to download host");

        assert!(
            find_wasmcloud_binary(&download_dir, "v0.63.1")
                .await
                .is_some(),
            "wasmCloud should be installed"
        );

        // Just to triple check, make sure the paths actually exist
        assert!(
            download_dir.join(WASMCLOUD_VERSION).exists(),
            "Directory should exist"
        );
        assert!(
            download_dir.join("v0.63.1").exists(),
            "Directory should exist"
        );

        let _ = remove_dir_all(download_dir).await;
    }

    const NATS_SERVER_VERSION: &str = "v2.8.4";
    const WASMCLOUD_HOST_VERSION: &str = "v0.63.1";

    #[tokio::test]
    async fn can_download_and_start_wasmcloud() -> anyhow::Result<()> {
        #[cfg(target_family = "unix")]
        let install_dir = temp_dir().join("can_download_and_start_wasmcloud");
        // This is a very specific hack to download wasmCloud to the same _drive_ on Windows
        // Turns out the mix release .bat file can't support executing an application that's installed
        // on a different drive (e.g. running wasmCloud on the D: drive from the C: drive), which is what
        // GitHub Actions does by default (runs in the D: drive, creates temp dir in the C: drive)
        #[cfg(target_family = "windows")]
        let install_dir = std::env::current_dir()?.join("can_download_and_start_wasmcloud");
        let _ = remove_dir_all(&install_dir).await;
        create_dir_all(&install_dir).await?;
        assert!(find_wasmcloud_binary(&install_dir, WASMCLOUD_HOST_VERSION)
            .await
            .is_none());

        // Install and start NATS server for this test
        let nats_port = 10004;
        assert!(ensure_nats_server(NATS_SERVER_VERSION, &install_dir)
            .await
            .is_ok());
        assert!(is_bin_installed(&install_dir, NATS_SERVER_BINARY).await);
        let config = NatsConfig::new_standalone("127.0.0.1", nats_port, None);
        let mut nats_child = start_nats_server(
            install_dir.join(NATS_SERVER_BINARY),
            std::process::Stdio::null(),
            config,
        )
        .await
        .expect("Unable to start nats process");

        let wasmcloud_binary = ensure_wasmcloud(WASMCLOUD_HOST_VERSION, &install_dir)
            .await
            .expect("Unable to ensure wasmcloud");

        let stderr_log_path = wasmcloud_binary
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("wasmcloud_stderr.log");
        let stderr_log_file = tokio::fs::File::create(&stderr_log_path)
            .await?
            .into_std()
            .await;
        let stdout_log_path = wasmcloud_binary
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("wasmcloud_stdout.log");
        let stdout_log_file = tokio::fs::File::create(&stdout_log_path)
            .await?
            .into_std()
            .await;

        let mut host_env = HashMap::new();
        host_env.insert("WASMCLOUD_DASHBOARD_PORT".to_string(), "5003".to_string());
        host_env.insert("WASMCLOUD_RPC_PORT".to_string(), nats_port.to_string());
        host_env.insert("WASMCLOUD_CTL_PORT".to_string(), nats_port.to_string());
        host_env.insert("WASMCLOUD_PROV_RPC_PORT".to_string(), nats_port.to_string());
        let mut host_child = start_wasmcloud_host(
            &wasmcloud_binary,
            stdout_log_file,
            stderr_log_file,
            host_env,
        )
        .await
        .expect("Unable to start wasmcloud host");

        // Give wasmCloud max 15 seconds to start up
        for _ in 0..14 {
            let log_contents = tokio::fs::read_to_string(&stderr_log_path).await?;
            if log_contents.is_empty() {
                println!("wasmCloud hasn't started up yet, waiting 1 second");
                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
            } else {
                // Give just a little bit of time for the startup logs to flow in, re-read logs
                tokio::time::sleep(std::time::Duration::from_millis(5000)).await;
                let log_contents = tokio::fs::read_to_string(&stderr_log_path).await?;
                assert!(log_contents
                    .contains("connect to control interface NATS without authentication"));
                assert!(log_contents.contains("connect to lattice rpc NATS without authentication"));
                assert!(log_contents.contains("Started wasmCloud OTP Host Runtime"));
                break;
            }
        }

        // Should fail because the port is already in use by another host
        let mut host_env = HashMap::new();
        host_env.insert("WASMCLOUD_DASHBOARD_PORT".to_string(), "5003".to_string());
        host_env.insert("WASMCLOUD_RPC_PORT".to_string(), nats_port.to_string());
        host_env.insert("WASMCLOUD_CTL_PORT".to_string(), nats_port.to_string());
        host_env.insert("WASMCLOUD_PROV_RPC_PORT".to_string(), nats_port.to_string());
        start_wasmcloud_host(
            &wasmcloud_binary,
            std::process::Stdio::null(),
            std::process::Stdio::null(),
            host_env,
        )
        .await
        .expect_err("Starting a second process should error");

        // Burrito releases (0.63.0+) do support multiple hosts, so this should work fine
        let mut host_env = HashMap::new();
        host_env.insert("WASMCLOUD_DASHBOARD_PORT".to_string(), "4002".to_string());
        host_env.insert("WASMCLOUD_RPC_PORT".to_string(), nats_port.to_string());
        host_env.insert("WASMCLOUD_CTL_PORT".to_string(), nats_port.to_string());
        host_env.insert("WASMCLOUD_PROV_RPC_PORT".to_string(), nats_port.to_string());
        let child_res = start_wasmcloud_host(
            &wasmcloud_binary,
            std::process::Stdio::null(),
            std::process::Stdio::null(),
            host_env,
        )
        .await;
        assert!(child_res.is_ok());
        child_res.unwrap().kill().await?;

        host_child.kill().await?;
        nats_child.kill().await?;
        let _ = remove_dir_all(install_dir).await;
        Ok(())
    }

    #[tokio::test]
    async fn can_properly_deny_elixir_release_hosts() -> anyhow::Result<()> {
        // Ensure we allow versions >= 0.63.0
        assert!(check_version("v1.56.0").is_ok());
        assert!(check_version("v0.63.0").is_ok());
        assert!(check_version("v0.63.1").is_ok());
        assert!(check_version("v0.63.2").is_ok());
        assert!(check_version("v0.64.0").is_ok());
        assert!(check_version("v0.100.0").is_ok());
        assert!(check_version("v0.203.0").is_ok());

        // Ensure we allow prerelease tags for testing
        assert!(check_version("v0.64.0-rc.1").is_ok());
        assert!(check_version("v0.64.0-alpha.23").is_ok());
        assert!(check_version("v0.64.0-beta.0").is_ok());

        // Ensure we deny versions < 0.63.0
        assert!(check_version("v0.48.0").is_err());
        assert!(check_version("v0.56.0").is_err());
        assert!(check_version("v0.58.0").is_err());
        assert!(check_version("v0.62.3").is_err());
        assert!(check_version("v0.12.0").is_err());
        assert!(check_version("v0.56.999").is_err());
        if let Err(e) = check_version("v0.56.0") {
            assert_eq!(e.to_string(), "wasmCloud version v0.56.0 is earlier than the minimum supported version of v0.63.0");
        } else {
            panic!("v0.56.0 should be before the minimum version")
        }

        // The check_version will allow bad semantic versions, rather than failing immediately
        assert!(check_version("ungabunga").is_ok());
        assert!(check_version("v11.1").is_ok());

        Ok(())
    }
}
