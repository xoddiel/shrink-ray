use std::ffi::OsStr;
use std::process::Output;
use std::{collections::HashMap, path::Path};
use std::collections::hash_map::Entry;
use std::env;
use std::path::PathBuf;
use magic::{Cookie, CookieFlags};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tracing::{debug, trace};

use crate::options::OutputOptions;
use crate::terminal::Terminal;

pub struct Context {
	binaries: HashMap<&'static str, PathBuf>,
	cookie: Cookie,
	pub terminal: Terminal,
	pub output_options: OutputOptions
}

impl Context {
	pub async fn new(terminal: Terminal, output_options: OutputOptions) -> Result<Self, crate::Error> {
		trace!("initializing libmagic");
		let cookie = Cookie::open(CookieFlags::MIME_TYPE | CookieFlags::ERROR)?;

		trace!("loading libmagic database");
		// TODO: load databases manually using tokio
		cookie.load::<&str>(&[])?;

		let binaries = HashMap::new();
		Ok(Self { binaries, cookie, terminal, output_options })
	}

	pub async fn get_output_file(&self, input: impl AsRef<Path>, suffix: impl AsRef<OsStr>) -> Result<PathBuf, crate::Error> {
		let output = self.output_options.get(input, suffix);
		if let Some(parent) = output.parent() {
			if !parent.exists() {
				fs::create_dir_all(parent).await?;
			}
		}

		Ok(output)
	}

	pub fn command(&mut self, name: &'static str) -> Result<Command, crate::Error> {
		let path = match self.binaries.entry(name) {
			Entry::Occupied(x) => x.into_mut().as_path(),
			Entry::Vacant(x) => {
				let path = match Self::probe_env(name)? {
					Some(x) => x,
					None => Self::probe_system(name)?
				};

				x.insert(path).as_path()
			}
		};

		Ok(Command::new(path))
	}

	#[cfg(target_family = "unix")]
	pub async fn run(&mut self, mut command: Command, input: impl AsRef<Path>) -> Result<Output, crate::Error> {
		use std::process::Stdio;
		use std::time::Duration;
		use nix::sys::signal::{kill, Signal};
		use nix::unistd::Pid;
		use tokio::io::{AsyncBufReadExt, BufReader};
		use tokio::signal;
		use tokio::time::{self, interval};

		let input = input.as_ref();
		command
			.stdin(Stdio::null())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped());

		debug!("spawning {:?}", command);
		let mut child = command.spawn()?;
		debug!("spawned {:?}", child);

		let mut out_buffer = BufReader::new(child.stdout.take().unwrap());
		let mut stdout = Vec::new();

		let mut err_buffer = BufReader::new(child.stderr.take().unwrap());
		let mut stderr = Vec::new();

		let mut progress = 0;
		let mut cancel = false;
		self.terminal.start_processing(input);

		let mut interval = interval(Duration::from_millis(100));
		interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

		loop {
			tokio::select! {
				status = child.wait() => {
					let status = status?;
					debug!("child process {}", status);
					self.terminal.end_processing();
					if cancel {
						return Err(crate::Error::Cancelled)
					}

					return Ok(Output { status, stdout, stderr });
				},

				_ = interval.tick() => {
					progress += 1;
					self.terminal.update_processing(input, progress, cancel);
				},

				result = err_buffer.read_until(b'\n', &mut stderr) => {
					let _ = result?;
					let err = String::from_utf8_lossy(stderr.as_ref());
					self.terminal.write_processing(input, progress, cancel, err);
					stderr.clear();
				},

				result = out_buffer.read_until(b'\n', &mut stdout) => {
					let _ = result?;
					let out = String::from_utf8_lossy(stdout.as_ref());
					self.terminal.write_processing(input, progress, cancel, out);
					stdout.clear();
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
							self.terminal.end_processing();
							return Err(crate::Error::from(errno));
						}
					}
				}
			}
		}
	}

	pub async fn identify_file(&self, path: impl AsRef<Path>) -> Result<Option<String>, crate::Error> {
    	let path = path.as_ref();
    	trace!("identifying file `{}`", path.display());

    	let mut buffer = [0; 1024];
    	let mut f = OpenOptions::new().read(true).open(path).await?;
    	let count = f.read(&mut buffer).await?;
    	let mime = self.identify_buffer(&buffer[..count])?;

    	if let Some(mime) = mime.as_deref() {
    		debug!("identified file `{}` as `{}`", path.display(), mime);
    	} else {
    		debug!("unable to identify file `{}`", path.display());
    	}

    	Ok(mime)
	}

	fn identify_buffer(&self, buffer: impl AsRef<[u8]>) -> Result<Option<String>, crate::Error> {
    	let buffer = buffer.as_ref();
    	trace!("identifying {} bytes using libmagic", buffer.len());
    	let mime = self.cookie.buffer(buffer)?;
    	debug!("libmagic returned `{}`", mime);

    	if !mime.contains('/') {
    		trace!(
    			"returned value `{}` does not seem to be a MIME type; assuming file failed to be identified",
    			mime
    		);
    		return Ok(None);
    	}

    	Ok(Some(mime))
	}

	fn probe_env(name: &'static str) -> Result<Option<PathBuf>, crate::Error> {
		let var_name = format!("RAY_BIN_{}", name.to_ascii_uppercase());
		trace!("checking for binary `{}` in environment (`{}`)...", name, var_name);

		let Some(path) = env::var_os(var_name.as_str()) else {
			return Ok(None);
		};

		let path = PathBuf::from(path);
		if !path.exists() {
			return Err(crate::Error::BinaryInEnvNotFound(path));
		}

		Ok(Some(path))
	}

	fn probe_system(name: &'static str) -> Result<PathBuf, crate::Error> {
		trace!("probing for `{}` binary...", name);
		match which::which(name) {
			Ok(x) => {
				debug!("found binary `{}`", x.display());

				Ok(x)
			}

			Err(which::Error::CannotFindBinaryPath) => Err(crate::Error::BinaryNotFound(name)),
			Err(x) => Err(crate::Error::from(x)),
		}
	}
}
