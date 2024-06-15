use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use clap::Parser;
use tracing::{debug, trace};

use crate::temp;

#[derive(Debug, Parser)]
#[command(author, version, about)]
pub struct Options {
	/// Files to convert
	#[arg(required = true)]
	pub inputs: Vec<PathBuf>,
	/// Output options
	#[command(flatten)]
	pub output: Output,
	/// Discard output file if it ended up being bigger than the input file
	#[arg(short = 'G', long)]
	pub no_grow: bool,
	/// Print the conversion command, but do not run it
	#[arg(short = 'n', long)]
	pub dry_run: bool,
}

#[derive(Debug, clap::Args)]
#[group(required = false, multiple = false)]
pub struct Output {
	/// Output file
	#[arg(short = 'o', long = "output-file", value_name = "PATH")]
	pub file: Option<PathBuf>,
	/// Output file without extension
	#[arg(short, long = "output-prefix", value_name = "PATH")]
	pub prefix: Option<PathBuf>,
	/// Output directory
	#[arg(short, long = "output-dir", value_name = "PATH")]
	pub dir: Option<PathBuf>,
}

impl Output {
	pub fn should_replace(&self) -> bool {
		matches!(
			self,
			Output {
				file: None,
				prefix: None,
				dir: None
			}
		)
	}

	pub fn get(&self, input: impl AsRef<Path>, extension: impl AsRef<OsStr>) -> PathBuf {
		if let Some(file) = &self.file {
			return file.clone();
		}

		if let Some(prefix) = &self.prefix {
			let mut prefix = prefix.clone().into_os_string();
			prefix.push(".");
			prefix.push(extension);
			return prefix.into();
		}

		if let Some(dir) = &self.dir {
			return dir.join(input.as_ref().file_name().unwrap()).with_extension(extension);
		}

		trace!("no output file given; choosing random temporary file");
		let name = temp::file(&input, Some(extension.as_ref()));
		debug!("chose a temporary output file `{}`", name.display());
		name
	}
}
