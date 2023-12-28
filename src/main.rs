use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, ExitStatus};

use clap::Parser;
use rand::Rng;
use tokio::process::Command;
use tokio::{fs, io, signal};
use tracing::{debug, error, trace};
use tracing_subscriber::EnvFilter;
use size::Size;

use crate::shrink::{Shrink, ShrinkTool};

mod shrink;

#[macro_use]
extern crate derive_more;

#[derive(Debug, Display, Error, From)]
pub enum Error {
	#[display(fmt = "input file `{}` not found", "_0.display()")]
	#[error(ignore)]
	#[from(ignore)]
	InputNotFound(PathBuf),
	#[display(fmt = "output file `{}` already exists", "_0.display()")]
	#[error(ignore)]
	#[from(ignore)]
	OutputExists(PathBuf),
	#[display(fmt = "tool `{}` not found", _0)]
	#[error(ignore)]
	#[from(ignore)]
	ToolNotFound(&'static str),
	#[display(fmt = "{} invocation failed, {}", _0, _1)]
	#[error(ignore)]
	#[from(ignore)]
	Conversion(&'static str, ExitStatus),
	Io(io::Error),
	Which(which::Error),
	#[cfg(target_family = "unix")]
	Nix(nix::errno::Errno),
}

#[derive(Debug, Parser)]
#[command(author, version, about)]
pub struct Args {
	/// Input file to convert
	input: PathBuf,
	/// Output file (will replace input file if not given)
	output: Option<PathBuf>,
	/// Discard output file if it ended up being bigger than the input file
	#[arg(short = 'G', long)]
	no_grow: bool
}

#[tokio::main]
async fn main() -> ExitCode {
	tracing_subscriber::fmt()
		.with_env_filter(EnvFilter::from_default_env())
		.init();

	let args = Args::parse();
	debug!("arguments: {:?}", args);

	match run(args).await {
		Ok(_) => ExitCode::SUCCESS,
		Err(x) => {
			eprintln!("{}", x);
			ExitCode::FAILURE
		}
	}
}

async fn run(args: Args) -> Result<(), Error> {
	if !args.input.exists() {
		return Err(Error::InputNotFound(args.input));
	}

	let Some(tool) = ShrinkTool::for_file(&args.input).await? else {
		return Ok(());
	};

	let mut swap_files = false;
	let output = args.output.unwrap_or_else(|| {
		trace!("no output file given; choosing random temporary file");
		let extension = tool.get_default_extension(&args.input);
		let name = temp_file(&args.input, Some(AsRef::<OsStr>::as_ref(extension)));
		debug!("chose a temporary output file `{}`", name.display());
		swap_files = true;
		name
	});

	if let Some(mut dir) = output.parent() {
		if dir.as_os_str().is_empty() {
			dir = AsRef::<Path>::as_ref(".");
		}

		if !dir.is_dir() {
			trace!("creating output directory `{}`", dir.display());
			fs::create_dir_all(dir).await?;
		}
	}

	let Err(error) = tool.shrink(&args.input, &output).await else {
		let input_meta = fs::metadata(&args.input).await?;
		let output_meta = fs::metadata(&output).await?;

		let input_size = input_meta.len();
		let output_size = output_meta.len();
		if output_size <= input_size {
			println!("shrunk {} to {} (saved {}, -{:.2} %)", Size::from_bytes(input_size), Size::from_bytes(output_size), Size::from_bytes(input_size - output_size), 100.0 * (input_size - output_size) as f64 / input_size as f64);
		} else {
			println!("grew {} to {} (wasted {}, +{:.2} %)", Size::from_bytes(input_size), Size::from_bytes(output_size), Size::from_bytes(output_size - input_size), 100.0 * (output_size - input_size) as f64 / input_size as f64);
			if args.no_grow {
				trace!("conversion grew file, removing `{}`", output.display());
				fs::remove_file(output).await?;
				return Ok(());
			}
		}

		filetime::set_file_mtime(&output, filetime::FileTime::from_last_modification_time(&input_meta))?;

		if !swap_files {
			return Ok(());
		}

		return replace(args.input, output).await;
	};

	if output.is_file() {
		trace!("conversion failed, removing `{}`", output.display());
		if let Err(error) = fs::remove_file(output).await {
			error!("failed to remove output file: {}", error);
		}
	}

	Err(error)
}

fn temp_file(path: impl AsRef<Path>, extension: Option<&OsStr>) -> PathBuf {
	const CHARS: &str = "abcdefghijklmnopqrstuvwxyz0123456789";
	const LENGTH: usize = 4;

	let path = path.as_ref();
	let mut rng = rand::thread_rng();
	let mut prefix = path.parent().unwrap().join(path.file_stem().unwrap()).into_os_string();
	prefix.push("-");
	loop {
		let mut buf = prefix.clone();

		for _ in 0..LENGTH {
			let index = rng.gen_range(0..CHARS.len());
			buf.push(&CHARS[index..=index]);
		}

		if let Some(extension) = extension {
			buf.push(".");
			buf.push(extension);
		}

		let path = PathBuf::from(buf);
		if !path.exists() {
			return path;
		}
	}
}

#[cfg(target_family = "unix")]
async fn run_command(mut command: Command) -> Result<ExitStatus, Error> {
	use nix::sys::signal::{kill, Signal};
	use nix::unistd::Pid;

	debug!("spawning {:?}", command);
	let mut child = command.spawn()?;
	debug!("spawned {:?}", child);

	loop {
		tokio::select! {
			status = child.wait() => {
				let status = status?;
				debug!("child process {}", status);
				return Ok(status);
			},

			_ = signal::ctrl_c() => {
				trace!("forwarding SIGINT");
				if let Some(id) = child.id() {
					let Err(errno) = kill(Pid::from_raw(id as i32), Signal::SIGINT) else {
						continue;
					};

					if errno == nix::errno::Errno::ESRCH {
						continue;
					} else {
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

	let temp = temp_file(input, input.extension());
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
