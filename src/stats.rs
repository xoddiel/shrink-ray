use size::Size;

#[derive(Copy, Clone, Debug, Default)]
pub struct Statistics {
	processed: u64,
	saved: u64,
	wasted: u64,
	shrunk: usize,
	grew: usize,
	skipped: usize,
	failed: usize,
}

impl Statistics {
	pub fn shrink(&mut self, delta: Delta) {
		self.processed += delta.original;
		self.saved += delta.difference();
		self.shrunk += 1;
	}

	pub fn grow(&mut self, delta: Delta) {
		self.processed += delta.original;
		self.wasted += delta.difference();
		self.grew += 1;
	}

	pub fn skip(&mut self) {
		self.skipped += 1;
	}

	pub fn fail(&mut self) {
		self.failed += 1;
	}

	pub fn shrunk_files(&self) -> usize {
		self.shrunk
	}

	pub fn saved_size(&self) -> Size {
		Size::from_bytes(self.saved)
	}

	pub fn grew_files(&self) -> usize {
		self.grew
	}

	pub fn wasted_size(&self) -> Size {
		Size::from_bytes(self.wasted)
	}

	pub fn delta(&self) -> Delta {
		Delta::new(self.processed, self.processed - self.saved + self.wasted)
	}

	pub fn skipped_files(&self) -> usize {
		self.skipped
	}

	pub fn failed_files(&self) -> usize {
		self.failed
	}
}

#[derive(Copy, Clone, Debug)]
pub struct Delta {
	pub original: u64,
	pub new: u64,
}

impl Delta {
	pub fn new(original: u64, new: u64) -> Self {
		Delta { original, new }
	}

	pub fn is_smaller(&self) -> bool {
		self.original >= self.new
	}

	pub fn original_size(&self) -> Size {
		Size::from_bytes(self.original)
	}

	pub fn new_size(&self) -> Size {
		Size::from_bytes(self.new)
	}

	pub fn size_difference(&self) -> Size {
		Size::from_bytes(self.difference())
	}

	pub fn difference(&self) -> u64 {
		if self.original > self.new {
			self.original - self.new
		} else {
			self.new - self.original
		}
	}

	pub fn ratio(&self) -> f64 {
		self.difference() as f64 / self.original as f64
	}
}
