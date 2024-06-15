use std::io::{stdout, Write};
use std::path::Path;
use std::process::{ExitCode, ExitStatus, Stdio};
use std::time::Duration;

use clap::{CommandFactory, Parser};
use crossterm::style::Stylize;
use crossterm::{cursor, terminal};
use delta::Delta;
use error::Error;
use options::Options;
use shrink::{Shrink, ShrinkTool};
use size::Size;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::{fs, signal, time};
use tracing::{debug, error, trace};
use tracing_subscriber::EnvFilter;

mod delta;
mod error;
mod options;
mod shrink;
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

	let mut processed = 0;
	let mut saved = 0;
	let mut wasted = 0;
	let mut shrunk = 0;
	let mut grew = 0;
	let mut skipped = 0;
	let mut failed = 0;
	for input in &options.inputs {
		match run_input(input, &options).await {
			Ok(delta) if delta.is_smaller() => {
				let stats = format!("(-{}, -{:.2} %)", delta.size_difference(), 100.0 * delta.ratio());
				println!("      {} {} {}", "Shrunk".green().bold(), input.display(), stats.dim());
				processed += delta.original;
				saved += delta.difference();
				shrunk += 1;
			}
			Ok(delta) => {
				let stats = format!("(+{}, +{:.2} %)", delta.size_difference(), 100.0 * delta.ratio());
				println!(
					"        {} {} {}",
					"Grew".dark_yellow().bold(),
					input.display(),
					stats.dim()
				);
				processed += delta.original;
				wasted += delta.difference();
				grew += 1;
			}
			Err(Error::InputFormatUnknown(_)) => {
				println!(
					"     {} {} {}",
					"Skipped".magenta().bold(),
					input.display(),
					"(unknown file format)".dim()
				);
				skipped += 1;
			}
			Err(Error::Conversion(_, status)) => {
				// TODO: extract prints into their own functions/module?
				let status = format!("({})", status);
				println!("      {} {} {}", "Failed".red().bold(), input.display(), status.dim());
				failed += 1;

				if !options.keep_going {
					break;
				}
			}
			Err(x) => {
				eprintln!("{}", x);
				return ExitCode::FAILURE;
			}
		}
	}

	if options.stats {
		println!();
		print!(
			"{} {} {}, ",
			"Shrunk".green().bold(),
			shrunk,
			format!("(-{})", Size::from_bytes(saved)).dim()
		);
		print!(
			"{} {} {}, ",
			"Grew".dark_yellow().bold(),
			grew,
			format!("(+{})", Size::from_bytes(wasted)).dim()
		);
		print!("{} {}, ", "Skipped".magenta().bold(), skipped);
		println!("{} {} ", "Failed".red().bold(), failed);

		let delta = Delta::new(processed, processed - saved + wasted);
		print!("Processed {}, ", delta.original_size());
		if saved > wasted {
			let ratio = format!("(-{:.2} %)", delta.ratio());
			println!(
				"{} -{} {}",
				"saving".green().bold(),
				delta.size_difference(),
				ratio.dim()
			);
		} else {
			let ratio = format!("(+{:.2} %)", delta.ratio());
			println!(
				"{} +{} {}",
				"wasting".dark_yellow().bold(),
				delta.size_difference(),
				ratio.dim()
			);
		}

		println!();
	}

	if failed > 0 {
		ExitCode::FAILURE
	} else {
		ExitCode::SUCCESS
	}
}

async fn run_input(input: impl AsRef<Path>, args: &Options) -> Result<Delta, Error> {
	let input = input.as_ref();
	if !input.exists() {
		return Err(Error::InputNotFound(input.to_path_buf()));
	}

	if input.is_symlink() {
		// TODO: handle symlinks
		return Err(Error::InputIsSymlink(input.to_path_buf()));
	}

	let Some(tool) = ShrinkTool::for_file(input).await? else {
		return Err(Error::InputFormatUnknown(input.to_path_buf()));
	};

	let output = args.output.get(input, tool.extension(input));
	if let Some(mut dir) = output.parent() {
		if dir.as_os_str().is_empty() {
			dir = AsRef::<Path>::as_ref(".");
		}

		if !dir.is_dir() {
			trace!("creating output directory `{}`", dir.display());
			fs::create_dir_all(dir).await?;
		}
	}

	let delta = run_tool(tool, input, &output).await?;
	if args.no_grow && !delta.is_smaller() {
		trace!("conversion grew file, removing `{}`", output.display());
		fs::remove_file(output).await?;
		return Ok(delta);
	}

	// TODO: rotate files when output is explicitly given, but it coincides with
	// input
	if args.output.should_replace() {
		replace(input, output).await?;
	}

	Ok(delta)
}

async fn run_tool(tool: ShrinkTool, input: impl AsRef<Path>, output: impl AsRef<Path>) -> Result<Delta, Error> {
	let input = input.as_ref();
	let output = output.as_ref();
	if let Err(error) = execute_tool(tool, input, output).await {
		if output.is_file() {
			trace!("conversion failed, removing `{}`", output.display());
			if let Err(error) = fs::remove_file(output).await {
				error!("failed to remove output file: {}", error);
			}
		}

		return Err(error);
	}

	let input_meta = fs::metadata(input).await?;
	let output_meta = fs::metadata(output).await?;

	let input_size = input_meta.len();
	let output_size = output_meta.len();
	filetime::set_file_mtime(output, filetime::FileTime::from_last_modification_time(&input_meta))?;

	Ok(Delta::new(input_size, output_size))
}

async fn execute_tool(tool: ShrinkTool, input: impl AsRef<Path>, output: impl AsRef<Path>) -> Result<(), Error> {
	let input = input.as_ref();
	let mut command = tool.command(input, output);
	command
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::piped());

	let status = execute_command(command, input).await?;
	if !status.success() {
		return Err(Error::Conversion(tool.name(), status));
	}

	Ok(())
}

// TODO: add support for Windows as well

#[cfg(target_family = "unix")]
async fn execute_command(mut command: Command, input: &Path) -> Result<ExitStatus, Error> {
	use nix::sys::signal::{kill, Signal};
	use nix::unistd::Pid;

	const ANIMATION: &[&str] = &["⠋", "⠙", "⠸", "⠴", "⠦", "⠇"];

	debug!("spawning {:?}", command);
	let mut child = command.spawn()?;
	debug!("spawned {:?}", child);

	let mut err = BufReader::new(child.stderr.take().unwrap());
	let mut err_buffer = Vec::new();

	let mut out = stdout().lock();
	let mut progress = 0;
	let mut cancel = false;
	let _ = write!(
		out,
		"   {} {} {}",
		"Shrinking".cyan().bold(),
		ANIMATION[progress],
		input.display()
	);
	let _ = out.flush();

	loop {
		tokio::select! {
			status = child.wait() => {
				let status = status?;
				debug!("child process {}", status);
				let _ = write!(out, "{}{}", cursor::MoveToColumn(0), terminal::Clear(terminal::ClearType::CurrentLine));
				let _ = out.flush();
				return Ok(status);
			},

			_ = time::sleep(Duration::from_millis(100)) => {
				progress = (progress + 1) % ANIMATION.len();
				let _ = write!(out, "{}{}", cursor::MoveToColumn(0), terminal::Clear(terminal::ClearType::UntilNewLine));
				let _ = if !cancel {
					write!(out, "   {} {} {}", "Shrinking".cyan().bold(), ANIMATION[progress], input.display())
				} else {
					write!(out, "  {} {} {}", "Cancelling".red().bold(), ANIMATION[progress], input.display())
				};
				let _ = out.flush();
			},

			result = err.read_until(b'\n', &mut err_buffer) => {
				let _ = result?;
				let _ = write!(out, "{}{}", cursor::MoveToColumn(0), terminal::Clear(terminal::ClearType::UntilNewLine));
				let err = String::from_utf8_lossy(err_buffer.as_ref());
				let _ = write!(out, "             {}", err.dim());
				let _ = out.flush();
				err_buffer.clear();
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
						let _ = write!(out, "{}{}", cursor::MoveToColumn(0), terminal::Clear(terminal::ClearType::CurrentLine));
						let _ = out.flush();
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
