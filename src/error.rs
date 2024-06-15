use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;

#[derive(Debug, Error)]
pub enum Error {
	#[error("input file `{}` not found", .0.display())]
	InputNotFound(PathBuf),
	#[error("input file `{}` is a symlink", .0.display())]
	InputIsSymlink(PathBuf),
	#[error("output file `{}` already exists", .0.display())]
	OutputExists(PathBuf),
	#[error("tool `{}` not found", .0)]
	ToolNotFound(&'static str),
	#[error("{} invocation failed, {}", .0, .1)]
	Conversion(&'static str, ExitStatus),
	#[error("{}", .0)]
	Magic(String),
	#[error(transparent)]
	Io(#[from] io::Error),
	#[error(transparent)]
	Which(#[from] which::Error),
	#[cfg(target_family = "unix")]
	#[error(transparent)]
	Nix(#[from] nix::errno::Errno),
}

impl Error {
	pub(crate) fn from_magic(error: impl std::error::Error) -> Self {
		Self::Magic(error.to_string())
	}
}
