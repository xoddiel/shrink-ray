use std::path::Path;
use std::process::ExitCode;

use clap::{CommandFactory, Parser};
use comment::Comment;
use context::Context;
use error::Error;
use options::Options;
use terminal::Terminal;
use stats::{Delta, Statistics};
use tokio::fs;
use tracing::{debug, trace, warn};
use tracing_subscriber::EnvFilter;

mod error;
mod options;
mod terminal;
mod stats;
mod temp;
mod image;
mod video;
mod context;
mod comment;

#[macro_use]
extern crate thiserror;

#[tokio::main]
async fn main() -> ExitCode {
	tracing_subscriber::fmt()
		.with_env_filter(EnvFilter::from_default_env())
		.init();

	let options = Options::parse();
	if options.inputs.len() > 1 && options.output.file.is_some() {
		Options::command()
			.error(
				clap::error::ErrorKind::ArgumentConflict,
				"the argument '--output-file <PATH>' cannot be used with multiple inputs",
			)
			.exit();
	}

	debug!("arguments: {:?}", options);

	let terminal = Terminal::new();
	let mut context = match Context::new(terminal, options.output.clone()).await {
		Ok(x) => x,
		Err(x) => {
			eprintln!("{}", x);
			return ExitCode::FAILURE;
		}
	};

	let mut cancel = false;
	let mut stats = Statistics::default();
	for input in &options.inputs {
		match run_input(input, &options, &mut context).await {
			Ok(delta) if delta.is_smaller() => {
				context.terminal.write_shrink(input, delta);
				stats.shrink(delta);
			}
			Ok(delta) => {
				context.terminal.write_grow(input, delta);
				stats.grow(delta);
			}
			Err(Error::InputFormatUnknown(_)) => {
				context.terminal.write_skip(input, "unknown file format");
				stats.skip();
			}
			Err(Error::AlreadyConverted(_)) => {
				context.terminal.write_skip(input, "file already converted");
				stats.skip();
			}
			Err(Error::Invocation(_, status)) => {
				context.terminal.write_fail(input, status);
				stats.fail();

				if !options.keep_going {
					break;
				}
			}
			Err(Error::Cancelled) => {
				context.terminal.write_cancel(input);
				cancel = true;
				break;
			}
			Err(x) => {
				eprintln!("{}", x);
				return ExitCode::FAILURE;
			}
		}
	}

	if options.stats {
		println!();
		context.terminal.write_stats(stats);
		println!();
	}

	if stats.failed_files() > 0 {
		ExitCode::FAILURE
	} else if cancel {
		// this will stop tools like `xargs`
		ExitCode::from(u8::MAX)
	} else {
		ExitCode::SUCCESS
	}
}

async fn run_input(
	input_file: impl AsRef<Path>, args: &Options, context: &mut Context,
) -> Result<Delta, Error> {
	let input_file = input_file.as_ref();
	if !input_file.exists() {
		return Err(Error::InputNotFound(input_file.to_path_buf()));
	}

	if input_file.is_symlink() {
		// TODO: handle symlinks
		return Err(Error::InputIsSymlink(input_file.to_path_buf()));
	}

	let Some(mime) = context.identify_file(input_file).await? else {
		return Err(Error::InputFormatUnknown(input_file.to_path_buf()));
	};

	let output_file = if mime == "image/gif" {
		// TODO: check if GIF is single- or multi-frame
		warn!("GIF files are currently not supported");
		return Err(Error::InputFormatUnknown(input_file.to_path_buf()));
	} else if mime.starts_with("image/") {
		match image::get_comment(context, input_file).await {
			Ok(Some(x)) => {
				debug!("comment found: {}", x);
				return Err(Error::AlreadyConverted(x))
			},
			Ok(None) => {},
			Err(crate::Error::Comment(x)) => debug!("unable to parse comment: {}", x),
			Err(x) => return Err(x)
		};

		image::convert(context, Comment::default(), input_file).await?
	} else if mime.starts_with("video/") {
		match video::get_comment(context, input_file).await {
			Ok(Some(x)) => {
				debug!("comment found: {}", x);
				return Err(Error::AlreadyConverted(x))
			},
			Ok(None) => {},
			Err(crate::Error::Comment(x)) => debug!("unable to parse comment: {}", x),
			Err(x) => return Err(x)
		};

		video::convert(context, Comment::default(), input_file).await?
	} else {
		warn!("unsupported file format: {}", mime);
		return Err(Error::InputFormatUnknown(input_file.to_path_buf()));
	};

	let input_meta = fs::metadata(input_file).await?;
	let output_meta = fs::metadata(&output_file).await?;

	let input_size = input_meta.len();
	let output_size = output_meta.len();
	filetime::set_file_mtime(
		&output_file,
		filetime::FileTime::from_last_modification_time(&input_meta),
	)?;

	let delta = Delta::new(input_size, output_size);
	if args.no_grow && !delta.is_smaller() {
		trace!("conversion grew file, removing `{}`", output_file.display());
		fs::remove_file(output_file).await?;
		return Ok(delta);
	}

	// TODO: rotate files when output is explicitly given, but it coincides with
	// input
	if args.output.should_replace() {
		replace(input_file, output_file).await?;
	}

	Ok(delta)
}

async fn replace(input: impl AsRef<Path>, output: impl AsRef<Path>) -> Result<(), Error> {
	let input = input.as_ref();
	let output = output.as_ref();
	let destination = input.with_extension(output.extension().unwrap());
	debug!(
		"replacing `{}` with `{}` (as `{}`)",
		input.display(),
		output.display(),
		destination.display()
	);
	if input != destination && destination.exists() {
		return Err(Error::OutputExists(destination));
	}

	let temp = temp::file(input, input.extension());
	trace!("renaming original file `{}` to `{}`", input.display(), temp.display());
	fs::rename(input, &temp).await?;

	trace!(
		"renaming new file `{}` to `{}`",
		output.display(),
		destination.display()
	);
	fs::rename(output, destination).await?;

	trace!("deleting original file `{}`", temp.display());
	fs::remove_file(temp).await?;

	Ok(())
}
