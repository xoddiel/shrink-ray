mod ffmpeg;
mod gm;

use std::path::{Path, PathBuf};

pub use ffmpeg::FFMpeg;
pub use gm::Gm;
use infer::Type;
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
		let Some(format) = Self::get_format(input).await? else {
			return Ok(None);
		};

		let mime = format.mime_type();
		if mime == "image/gif" {
			// TODO: check if GIF is single- or multi-frame
			warn!("GIF files are currently not supported");
			Ok(None)
		} else if mime.starts_with("image/") {
			Self::which("gm").map(|i| Some(Self::Gm(Gm(i))))
		} else if mime.starts_with("video/") {
			Self::which("ffmpeg").map(|i| Some(Self::FFMpeg(FFMpeg(i))))
		} else {
			warn!("unsupported file format: {}", format);
			Ok(None)
		}
	}

	async fn get_format(path: impl AsRef<Path>) -> Result<Option<Type>, super::Error> {
		let path = path.as_ref();
		trace!("identifying `{}`", path.display());

		let mut f = OpenOptions::new().read(true).open(path).await?;
		let mut buffer = [0; 1024];
		let count = f.read(&mut buffer).await?;
		let Some(format) = infer::get(&buffer[..count]) else {
			warn!("failed to identify `{}`", path.display());
			return Ok(None);
		};

		debug!("identified file `{}` as `{}`", path.display(), format);

		Ok(Some(format))
	}

	fn which(name: &'static str) -> Result<PathBuf, super::Error> {
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
