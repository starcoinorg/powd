use anyhow::{Context, Result};
use std::io;
use std::path::Path;
use tokio::io::{ReadHalf, WriteHalf};

#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};

#[cfg(windows)]
use futures::StreamExt;
#[cfg(windows)]
use parity_tokio_ipc::{Connection as PipeConnection, Endpoint as PipeEndpoint};

#[cfg(unix)]
pub type LocalConnection = UnixStream;
#[cfg(windows)]
pub type LocalConnection = PipeConnection;

pub type LocalReadHalf = ReadHalf<LocalConnection>;
pub type LocalWriteHalf = WriteHalf<LocalConnection>;

pub struct LocalListener {
    #[cfg(unix)]
    inner: UnixListener,
    #[cfg(windows)]
    incoming: futures::stream::BoxStream<'static, io::Result<LocalConnection>>,
}

impl LocalListener {
    pub async fn accept(&mut self) -> io::Result<LocalConnection> {
        #[cfg(unix)]
        {
            let (stream, _) = self.inner.accept().await?;
            Ok(stream)
        }

        #[cfg(windows)]
        {
            match self.incoming.next().await {
                Some(result) => result,
                None => Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "local pipe listener closed",
                )),
            }
        }
    }
}

pub fn bind_local(endpoint: &Path) -> Result<LocalListener> {
    #[cfg(unix)]
    {
        let listener = UnixListener::bind(endpoint)
            .with_context(|| format!("bind unix socket {}", endpoint.display()))?;
        Ok(LocalListener { inner: listener })
    }

    #[cfg(windows)]
    {
        let incoming = PipeEndpoint::new(normalize_pipe_path(endpoint))
            .incoming()
            .context("bind local named pipe")?
            .boxed();
        Ok(LocalListener { incoming })
    }
}

pub async fn connect_local(endpoint: &Path) -> io::Result<LocalConnection> {
    #[cfg(unix)]
    {
        UnixStream::connect(endpoint).await
    }

    #[cfg(windows)]
    {
        PipeEndpoint::connect(normalize_pipe_path(endpoint)).await
    }
}

pub async fn cleanup_local_endpoint(endpoint: &Path) {
    #[cfg(unix)]
    {
        let _ = tokio::fs::remove_file(endpoint).await;
    }

    #[cfg(windows)]
    {
        let _ = endpoint;
    }
}

pub fn verify_local_peer(stream: &LocalConnection) -> Result<()> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::fd::AsRawFd;

        let fd = stream.as_raw_fd();
        let mut ucred = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let rc = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut ucred as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if rc != 0 {
            return Err(std::io::Error::last_os_error()).context("read peer credentials");
        }
        let expected_uid = unsafe { libc::geteuid() };
        if ucred.uid != expected_uid {
            anyhow::bail!(
                "reject peer uid {} on local api socket; expected {}",
                ucred.uid,
                expected_uid
            );
        }
    }

    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;

        let fd = stream.as_raw_fd();
        let mut peer_euid: libc::uid_t = 0;
        let mut peer_egid: libc::gid_t = 0;
        let rc = unsafe { libc::getpeereid(fd, &mut peer_euid, &mut peer_egid) };
        if rc != 0 {
            return Err(std::io::Error::last_os_error()).context("read peer credentials");
        }
        let expected_uid = unsafe { libc::geteuid() };
        if peer_euid != expected_uid {
            anyhow::bail!(
                "reject peer uid {} on local api socket; expected {}",
                peer_euid,
                expected_uid
            );
        }
    }

    #[cfg(windows)]
    {
        let _ = stream;
    }

    Ok(())
}

#[cfg(windows)]
fn normalize_pipe_path(path: &Path) -> String {
    let raw = path.as_os_str().to_string_lossy().trim().to_string();
    if raw.starts_with(r"\\.\pipe\") {
        return raw;
    }
    let normalized = raw.to_lowercase();
    let digest = fnv1a64(normalized.as_bytes());
    format!(r"\\.\pipe\powd-endpoint-{digest:016x}")
}

#[cfg(windows)]
fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = OFFSET;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}
