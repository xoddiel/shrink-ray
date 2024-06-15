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
	pub output: OutputFiles,
	/// Discard output file if it ended up being bigger than the input file
	#[arg(short = 'G', long)]
	pub no_grow: bool,
	/// Do not stop when an input fails to process
	#[arg(short, long)]
	pub keep_going: bool,
	/// Show statistics once all files are processed
	#[arg(short, long)]
	pub stats: bool,
}

#[derive(Debug, clap::Args)]
#[group(required = false, multiple = false)]
pub struct OutputFiles {
	/// Output file
	#[arg(short = 'o', long = "output-file", value_name = "PATH")]
	pub file: Option<PathBuf>,
	/// Output directory
	#[arg(short, long = "output-dir", value_name = "PATH")]
	pub dir: Option<PathBuf>,
}

impl OutputFiles {
	pub fn should_replace(&self) -> bool {
		matches!(self, OutputFiles { file: None, dir: None })
	}

	pub fn get(&self, input: impl AsRef<Path>, extension: impl AsRef<OsStr>) -> PathBuf {
		if let Some(file) = &self.file {
			return file.clone();
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
