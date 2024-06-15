use std::path::Path;

use magic::cookie::{Cookie, DatabasePaths, Flags, Load};
use tokio::fs::OpenOptions;
use tokio::io::AsyncReadExt;
use tracing::{debug, trace};

pub struct Identify(Cookie<Load>);

impl Identify {
	pub fn new() -> Result<Self, crate::Error> {
		trace!("initializing libmagic");
		let cookie = Cookie::open(Flags::MIME_TYPE | Flags::ERROR).map_err(super::Error::from_magic)?;

		trace!("loading libmagic database");
		// TODO: load databases manually using tokio
		let cookie = cookie
			.load(&DatabasePaths::default())
			.map_err(super::Error::from_magic)?;

		Ok(Identify(cookie))
	}

	pub async fn file(&self, path: impl AsRef<Path>) -> Result<Option<String>, crate::Error> {
		let path = path.as_ref();
		trace!("identifying file `{}`", path.display());

		let mut buffer = [0; 1024];
		let mut f = OpenOptions::new().read(true).open(path).await?;
		let count = f.read(&mut buffer).await?;
		let mime = self.bytes(&buffer[..count])?;

		if let Some(mime) = mime.as_deref() {
			debug!("identified file `{}` as `{}`", path.display(), mime);
		} else {
			debug!("unable to identify file `{}`", path.display());
		}

		Ok(mime)
	}

	pub fn bytes(&self, bytes: impl AsRef<[u8]>) -> Result<Option<String>, crate::Error> {
		let bytes = bytes.as_ref();
		trace!("identifying {} bytes using libmagic", bytes.len());
		let mime = self.0.buffer(bytes).map_err(super::Error::from_magic)?;
		debug!("libmagic returned `{}`", mime);

		if !mime.contains('/') {
			trace!(
				"returned value `{}` does not seem to be a MIME type; assuming file failed to be identified",
				mime
			);
			return Ok(None);
		}

		Ok(Some(mime))
	}
}
