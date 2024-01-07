mod ffmpeg;
mod gm;

use std::path::{Path, PathBuf};

pub use ffmpeg::FFMpeg;
pub use gm::Gm;
use tokio::fs::OpenOptions;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tracing::{debug, trace, warn};

pub trait Shrink {
	fn name(&self) -> &'static str;

	fn extension(&self, input: impl AsRef<Path>) -> &'static str;

	fn command(&self, input: impl AsRef<Path>, output: impl AsRef<Path>) -> Command;
}

#[derive(Debug)]
pub enum ShrinkTool {
	FFMpeg(FFMpeg),
	Gm(Gm),
}

impl ShrinkTool {
	pub async fn for_file(input: impl AsRef<Path>) -> Result<Option<Self>, super::Error> {
		// TODO: cache and reuse binary paths and libmagic cookie

		let Some(mime) = Self::get_format(input).await? else {
			return Ok(None);
		};

		if mime == "image/gif" {
			// TODO: check if GIF is single- or multi-frame
			warn!("GIF files are currently not supported");
			Ok(None)
		} else if mime.starts_with("image/") {
			Self::which("gm").map(|i| Some(Self::Gm(Gm(i))))
		} else if mime.starts_with("video/") {
			Self::which("ffmpeg").map(|i| Some(Self::FFMpeg(FFMpeg(i))))
		} else {
			warn!("unsupported file format: {}", mime);
			Ok(None)
		}
	}

	async fn get_format(path: impl AsRef<Path>) -> Result<Option<String>, super::Error> {
		use magic::cookie::{Cookie, DatabasePaths, Flags};

		let path = path.as_ref();
		trace!("identifying `{}`", path.display());

		let mut buffer = [0; 1024];
		let mut f = OpenOptions::new().read(true).open(path).await?;
		let count = f.read(&mut buffer).await?;

		trace!("initializing libmagic");
		let cookie = Cookie::open(Flags::MIME_TYPE | Flags::ERROR).map_err(super::Error::from_magic)?;

		trace!("loading libmagic database");
		// TODO: load databases manually using tokio
		let cookie = cookie
			.load(&DatabasePaths::default())
			.map_err(super::Error::from_magic)?;

		trace!("identifying {} bytes using libmagic", count);
		let mime = cookie.buffer(&buffer[..count]).map_err(super::Error::from_magic)?;
		debug!("libmagic returned `{}`", mime);

		if !mime.contains('/') {
			trace!("returned value does not seem to be MIME; assuming file failed to be identified");
			return Ok(None);
		}

		debug!("identified file `{}` as `{}`", path.display(), mime);

		Ok(Some(mime))
	}

	fn which(name: &'static str) -> Result<PathBuf, super::Error> {
		// TODO: check environment variables (`SHRINKRAY_<NAME>`)

		trace!("looking for `{}` binary", name);
		match which::which(name) {
			Ok(x) => {
				debug!("found binary `{}`", x.display());
				Ok(x)
			}

			Err(which::Error::CannotFindBinaryPath) => Err(super::Error::ToolNotFound(name)),
			Err(x) => Err(super::Error::from(x)),
		}
	}
}

impl Shrink for ShrinkTool {
	fn name(&self) -> &'static str {
		match self {
			ShrinkTool::FFMpeg(x) => x.name(),
			ShrinkTool::Gm(x) => x.name(),
		}
	}

	fn extension(&self, input: impl AsRef<Path>) -> &'static str {
		match self {
			ShrinkTool::FFMpeg(x) => x.extension(input),
			ShrinkTool::Gm(x) => x.extension(input),
		}
	}

	fn command(&self, input: impl AsRef<Path>, output: impl AsRef<Path>) -> Command {
		match self {
			ShrinkTool::FFMpeg(x) => x.command(input, output),
			ShrinkTool::Gm(x) => x.command(input, output),
		}
	}
}
