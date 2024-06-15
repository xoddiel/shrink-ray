use size::Size;

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
