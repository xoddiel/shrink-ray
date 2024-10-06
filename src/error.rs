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
	#[error("input file `{}` could not be identified", .0.display())]
	InputFormatUnknown(PathBuf),
	#[error("binary `{}` not found", .0)]
	BinaryNotFound(&'static str),
	#[error("binary `{}` not found", .0.display())]
	BinaryInEnvNotFound(PathBuf),
	#[error("{} invocation failed, {}", .0, .1)]
	Invocation(&'static str, ExitStatus),
	#[error("cancelled")]
	Cancelled,
	#[error(transparent)]
	Magic(#[from] magic::MagicError),
	#[error(transparent)]
	Io(#[from] io::Error),
	#[error(transparent)]
	Which(#[from] which::Error),
	#[cfg(target_family = "unix")]
	#[error(transparent)]
	Nix(#[from] nix::errno::Errno),
}
