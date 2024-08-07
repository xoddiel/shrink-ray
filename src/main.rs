use std::path::Path;
use std::process::{ExitCode, ExitStatus, Stdio};
use std::time::Duration;

use clap::{CommandFactory, Parser};
use error::Error;
use identify::Identify;
use options::Options;
use output::Output;
use shrink::{Shrink, ShrinkTool};
use stats::{Delta, Statistics};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::interval;
use tokio::{fs, signal, time};
use tracing::{debug, error, trace};
use tracing_subscriber::EnvFilter;

mod error;
mod identify;
mod options;
mod output;
mod shrink;
mod stats;
mod temp;

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
	// TODO: bring back --dry-run

	let identify = match Identify::new() {
		Ok(x) => x,
		Err(x) => {
			eprintln!("{}", x);
			return ExitCode::FAILURE;
		}
	};

	let mut cancel = false;
	let mut output = Output::new();
	let mut stats = Statistics::default();
	for input in &options.inputs {
		match run_input(input, &options, &mut output, &identify).await {
			Ok(delta) if delta.is_smaller() => {
				output.write_shrink(input, delta);
				stats.shrink(delta);
			}
			Ok(delta) => {
				output.write_grow(input, delta);
				stats.grow(delta);
			}
			Err(Error::InputFormatUnknown(_)) => {
				output.write_skip(input, "unknown file format");
				stats.skip();
			}
			Err(Error::Conversion(_, status)) => {
				output.write_fail(input, status);
				stats.fail();

				if !options.keep_going {
					break;
				}
			}
			Err(Error::Cancelled) => {
				output.write_cancel(input);
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
		output.write_stats(stats);
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
	input_file: impl AsRef<Path>, args: &Options, output: &mut Output, identify: &Identify,
) -> Result<Delta, Error> {
	let input_file = input_file.as_ref();
	if !input_file.exists() {
		return Err(Error::InputNotFound(input_file.to_path_buf()));
	}

	if input_file.is_symlink() {
		// TODO: handle symlinks
		return Err(Error::InputIsSymlink(input_file.to_path_buf()));
	}

	let Some(mime) = identify.file(input_file).await? else {
		return Err(Error::InputFormatUnknown(input_file.to_path_buf()));
	};

	let Some(tool) = ShrinkTool::for_mime(mime)? else {
		return Err(Error::InputFormatUnknown(input_file.to_path_buf()));
	};

	let output_file = args.output.get(input_file, tool.extension(input_file));
	if let Some(mut dir) = output_file.parent() {
		if dir.as_os_str().is_empty() {
			dir = AsRef::<Path>::as_ref(".");
		}

		if !dir.is_dir() {
			trace!("creating output directory `{}`", dir.display());
			fs::create_dir_all(dir).await?;
		}
	}

	let delta = run_tool(tool, input_file, &output_file, output).await?;
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

async fn run_tool(
	tool: ShrinkTool, input_file: impl AsRef<Path>, output_file: impl AsRef<Path>, output: &mut Output,
) -> Result<Delta, Error> {
	let input_file = input_file.as_ref();
	let output_file = output_file.as_ref();
	if let Err(error) = execute_tool(tool, input_file, output_file, output).await {
		if output_file.is_file() {
			if matches!(error, Error::Cancelled) {
				trace!("conversion cancelled, removing `{}`", output_file.display());
			} else {
				trace!("conversion failed, removing `{}`", output_file.display());
			}
			if let Err(error) = fs::remove_file(output_file).await {
				error!("failed to remove output file: {}", error);
			}
		}

		return Err(error);
	}

	let input_meta = fs::metadata(input_file).await?;
	let output_meta = fs::metadata(output_file).await?;

	let input_size = input_meta.len();
	let output_size = output_meta.len();
	filetime::set_file_mtime(
		output_file,
		filetime::FileTime::from_last_modification_time(&input_meta),
	)?;

	Ok(Delta::new(input_size, output_size))
}

async fn execute_tool(
	tool: ShrinkTool, input_file: impl AsRef<Path>, output_file: impl AsRef<Path>, output: &mut Output,
) -> Result<(), Error> {
	let input_file = input_file.as_ref();
	let mut command = tool.command(input_file, output_file);
	command
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped());

	let status = execute_command(command, input_file, output).await?;
	if !status.success() {
		return Err(Error::Conversion(tool.name(), status));
	}

	Ok(())
}

// TODO: add support for Windows as well

#[cfg(target_family = "unix")]
async fn execute_command(mut command: Command, input: &Path, output: &mut Output) -> Result<ExitStatus, Error> {
	use nix::sys::signal::{kill, Signal};
	use nix::unistd::Pid;

	debug!("spawning {:?}", command);
	let mut child = command.spawn()?;
	debug!("spawned {:?}", child);

	let mut out = BufReader::new(child.stdout.take().unwrap());
	let mut out_buffer = Vec::new();

	let mut err = BufReader::new(child.stderr.take().unwrap());
	let mut err_buffer = Vec::new();

	let mut progress = 0;
	let mut cancel = false;
	output.start_processing(input);

	let mut interval = interval(Duration::from_millis(100));
	interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

	loop {
		tokio::select! {
			status = child.wait() => {
				let status = status?;
				debug!("child process {}", status);
				output.end_processing();
				if cancel {
					return Err(Error::Cancelled)
				}

				return Ok(status);
			},

			_ = interval.tick() => {
				progress += 1;
				output.update_processing(input, progress, cancel);
			},

			result = err.read_until(b'\n', &mut err_buffer) => {
				let _ = result?;
				let err = String::from_utf8_lossy(err_buffer.as_ref());
				output.write_processing(input, progress, cancel, err);
				err_buffer.clear();
			},

			result = out.read_until(b'\n', &mut out_buffer) => {
				let _ = result?;
				let out = String::from_utf8_lossy(out_buffer.as_ref());
				output.write_processing(input, progress, cancel, out);
				out_buffer.clear();
			},

			_ = signal::ctrl_c() => {
				trace!("forwarding SIGINT");
				if let Some(id) = child.id() {
					cancel = true;
					let Err(errno) = kill(Pid::from_raw(id as i32), Signal::SIGINT) else {
						continue;
					};

					if errno == nix::errno::Errno::ESRCH {
						continue;
					} else {
						output.end_processing();
						return Err(Error::from(errno));
					}
				}
			}
		}
	}
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
