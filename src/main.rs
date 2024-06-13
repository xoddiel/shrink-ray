use std::ffi::OsStr;
use std::io::{stdout, Write};
use std::path::{Path, PathBuf};
use std::process::{ExitCode, ExitStatus, Stdio};
use std::time::Duration;

use clap::{CommandFactory, Parser};
use crossterm::style::Stylize;
use crossterm::{cursor, terminal};
use size::Size;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::{fs, io, signal, time};
use tracing::{debug, error, trace};
use tracing_subscriber::EnvFilter;

use crate::shrink::{Shrink, ShrinkTool};

mod shrink;
mod temp;

#[macro_use]
extern crate thiserror;

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

#[derive(Debug, Parser)]
#[command(author, version, about)]
pub struct Args {
	/// Files to convert
	#[arg(required = true)]
	inputs: Vec<PathBuf>,
	/// Output options
	#[command(flatten)]
	output: Output,
	/// Discard output file if it ended up being bigger than the input file
	#[arg(short = 'G', long)]
	no_grow: bool,
	/// Print the conversion command, but do not run it
	#[arg(short = 'n', long)]
	dry_run: bool,
}

#[derive(Debug, clap::Args)]
#[group(required = false, multiple = false)]
struct Output {
	/// Output file
	#[arg(short = 'o', long = "output-file", value_name = "PATH")]
	file: Option<PathBuf>,
	/// Output file without extension
	#[arg(short, long = "output-prefix", value_name = "PATH")]
	prefix: Option<PathBuf>,
	/// Output directory
	#[arg(short, long = "output-dir", value_name = "PATH")]
	dir: Option<PathBuf>,
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

#[tokio::main]
async fn main() -> ExitCode {
	tracing_subscriber::fmt()
		.with_env_filter(EnvFilter::from_default_env())
		.init();

	let args = Args::parse();
	if args.inputs.len() > 1 {
		if args.output.file.is_some() {
			Args::command()
				.error(
					clap::error::ErrorKind::ArgumentConflict,
					"the argument '--output-file <PATH>' cannot be used with multiple inputs",
				)
				.exit();
		} else if args.output.prefix.is_some() {
			Args::command()
				.error(
					clap::error::ErrorKind::ArgumentConflict,
					"the argument '--output-prefix <PATH>' cannot be used with multiple inputs",
				)
				.exit();
		}
	}

	debug!("arguments: {:?}", args);

	for input in &args.inputs {
		match run_input(input, &args).await {
			Ok(_) => {}
			Err(Error::Conversion(_, status)) => {
				// TODO: extract prints into their own functions/module?
				let status = format!("({})", status);
				println!("      {} {} {}", "Failed".red().bold(), input.display(), status.dim());
				return ExitCode::FAILURE;
			}
			Err(x) => {
				eprintln!("{}", x);
				return ExitCode::FAILURE;
			}
		}
	}

	ExitCode::SUCCESS
}

async fn run_input(input: impl AsRef<Path>, args: &Args) -> Result<(), Error> {
	let input = input.as_ref();
	if !input.exists() {
		return Err(Error::InputNotFound(input.to_path_buf()));
	}

	if input.is_symlink() {
		// TODO: handle symlinks
		return Err(Error::InputIsSymlink(input.to_path_buf()));
	}

	let Some(tool) = ShrinkTool::for_file(input).await? else {
		return Ok(());
	};

	let output = args.output.get(input, tool.extension(input));

	if args.dry_run {
		return print_command(tool.command(input, output));
	}

	if let Some(mut dir) = output.parent() {
		if dir.as_os_str().is_empty() {
			dir = AsRef::<Path>::as_ref(".");
		}

		if !dir.is_dir() {
			trace!("creating output directory `{}`", dir.display());
			fs::create_dir_all(dir).await?;
		}
	}

	let shrunk = run_tool(tool, input, &output).await?;
	if args.no_grow && !shrunk {
		trace!("conversion grew file, removing `{}`", output.display());
		fs::remove_file(output).await?;
		return Ok(());
	}

	// TODO: rotate files when output is explicitly given, but it coincides with
	// input
	if !args.output.should_replace() {
		return Ok(());
	}

	replace(input, output).await
}

fn print_command(command: Command) -> Result<(), Error> {
	let command = command.as_std();
	let mut stdout = stdout();
	stdout.write_all(command.get_program().as_encoded_bytes())?;
	for arg in command.get_args() {
		write!(stdout, " ")?;
		stdout.write_all(arg.as_encoded_bytes())?;
	}
	writeln!(stdout)?;

	Ok(())
}

async fn run_tool(tool: ShrinkTool, input: impl AsRef<Path>, output: impl AsRef<Path>) -> Result<bool, Error> {
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
	if output_size <= input_size {
		let stats = format!(
			"(-{}, -{:.2} %)",
			Size::from_bytes(input_size - output_size),
			100.0 * (input_size - output_size) as f64 / input_size as f64
		);
		println!("      {} {} {}", "Shrunk".green().bold(), input.display(), stats.dim());
	} else {
		let stats = format!(
			"(+{}, +{:.2} %)",
			Size::from_bytes(output_size - input_size),
			100.0 * (output_size - input_size) as f64 / input_size as f64
		);
		println!(
			"        {} {} {}",
			"Grew".dark_yellow().bold(),
			input.display(),
			stats.dim()
		);
	}

	filetime::set_file_mtime(output, filetime::FileTime::from_last_modification_time(&input_meta))?;

	Ok(input_size >= output_size)
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

	const ANIMATION: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

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

			_ = time::sleep(Duration::from_millis(200)) => {
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
				let _ = out.write_all(err_buffer.as_ref());
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
