use std::fmt;
use std::io::{stdout, StdoutLock, Write};
use std::path::Path;

use crossterm::cursor::MoveToColumn;
use crossterm::style::Stylize;
use crossterm::terminal::{Clear, ClearType};

use crate::stats::{Delta, Statistics};

macro_rules! safe_write {
	($($args:expr),*) => {
		{
			let result = write!($($args),*);
			if cfg!(debug_assertions) {
				result.unwrap();
			}
		}
	}
}

macro_rules! safe_writeln {
	($($args:expr),*) => {
		{
			let result = writeln!($($args),*);
			if cfg!(debug_assertions) {
				result.unwrap();
			}
		}
	}
}

macro_rules! safe_flush {
	($stream:expr) => {{
		let result = $stream.flush();
		if cfg!(debug_assertions) {
			result.unwrap();
		}
	}};
}

pub struct Terminal(StdoutLock<'static>);

impl Terminal {
	const ANIMATION: &'static [&'static str] = &["⠋", "⠙", "⠸", "⠴", "⠦", "⠇"];

	pub fn new() -> Self {
		Terminal(stdout().lock())
	}

	pub fn write_shrink(&mut self, file: impl AsRef<Path>, delta: Delta) {
		safe_writeln!(
			self.0,
			"      {} {} {}",
			"Shrunk".green().bold(),
			file.as_ref().display(),
			format!("(-{}, -{:.2} %)", delta.size_difference(), 100.0 * delta.ratio()).dim()
		);
	}

	pub fn write_grow(&mut self, file: impl AsRef<Path>, delta: Delta) {
		safe_writeln!(
			self.0,
			"        {} {} {}",
			"Grew".dark_yellow().bold(),
			file.as_ref().display(),
			format!("(+{}, +{:.2} %)", delta.size_difference(), 100.0 * delta.ratio()).dim()
		);
	}

	pub fn write_skip(&mut self, file: impl AsRef<Path>, reason: impl fmt::Display) {
		safe_writeln!(
			self.0,
			"     {} {} {}",
			"Skipped".magenta().bold(),
			file.as_ref().display(),
			format!("({})", reason).dim()
		);
	}

	pub fn write_fail(&mut self, file: impl AsRef<Path>, reason: impl fmt::Display) {
		safe_writeln!(
			self.0,
			"      {} {} {}",
			"Failed".red().bold(),
			file.as_ref().display(),
			format!("({})", reason).dim()
		);
	}

	pub fn write_cancel(&mut self, file: impl AsRef<Path>) {
		safe_writeln!(self.0, "   {} {}", "Cancelled".red().bold(), file.as_ref().display());
	}

	pub fn write_stats(&mut self, stats: Statistics) {
		safe_write!(
			self.0,
			"{} {} {}, ",
			"Shrunk".green().bold(),
			stats.shrunk_files(),
			format!("(-{})", stats.saved_size()).dim()
		);
		safe_write!(
			self.0,
			"{} {} {}, ",
			"Grew".dark_yellow().bold(),
			stats.grew_files(),
			format!("(+{})", stats.wasted_size()).dim()
		);
		safe_write!(self.0, "{} {}, ", "Skipped".magenta().bold(), stats.skipped_files());
		safe_writeln!(self.0, "{} {} ", "Failed".red().bold(), stats.failed_files());

		let delta = stats.delta();
		safe_write!(self.0, "Processed {}, ", delta.original_size());
		if delta.is_smaller() {
			let ratio = format!("(-{:.2} %)", 100.0 * delta.ratio());
			safe_writeln!(
				self.0,
				"{} -{} {}",
				"saving".green().bold(),
				delta.size_difference(),
				ratio.dim()
			);
		} else {
			let ratio = format!("(+{:.2} %)", 100.0 * delta.ratio());
			safe_writeln!(
				self.0,
				"{} +{} {}",
				"wasting".dark_yellow().bold(),
				delta.size_difference(),
				ratio.dim()
			);
		}
	}

	pub fn start_processing(&mut self, file: impl AsRef<Path>) {
		self.write_shrinking(file, 0);
		safe_flush!(self.0);
	}

	pub fn update_processing(&mut self, file: impl AsRef<Path>, progress: usize, cancel: bool) {
		safe_write!(self.0, "{}{}", MoveToColumn(0), Clear(ClearType::UntilNewLine));
		if cancel {
			self.write_cancelling(file, progress);
		} else {
			self.write_shrinking(file, progress);
		}

		safe_flush!(self.0);
	}

	pub fn write_processing(&mut self, file: impl AsRef<Path>, progress: usize, cancel: bool, line: impl AsRef<str>) {
		safe_write!(self.0, "{}{}", MoveToColumn(0), Clear(ClearType::UntilNewLine));
		let _ = write!(self.0, "             {}", line.as_ref().dim());
		if cancel {
			self.write_cancelling(file, progress);
		} else {
			self.write_shrinking(file, progress);
		}

		safe_flush!(self.0)
	}

	pub fn end_processing(&mut self) {
		safe_write!(self.0, "{}{}", MoveToColumn(0), Clear(ClearType::UntilNewLine));
		safe_flush!(self.0);
	}

	fn write_shrinking(&mut self, file: impl AsRef<Path>, progress: usize) {
		safe_write!(self.0, "   {} ", "Shrinking".cyan().bold());
		self.write_processing_file(file, progress)
	}

	fn write_cancelling(&mut self, file: impl AsRef<Path>, progress: usize) {
		safe_write!(self.0, "  {} ", "Cancelling".red().bold());
		self.write_processing_file(file, progress)
	}

	fn write_processing_file(&mut self, file: impl AsRef<Path>, progress: usize) {
		safe_write!(
			self.0,
			"{} {}",
			Self::ANIMATION[progress % Self::ANIMATION.len()],
			file.as_ref().display()
		);
	}
}
