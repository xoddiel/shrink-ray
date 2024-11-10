use core::fmt;
use std::str::FromStr;

use semver::Version;

#[derive(Debug, Error)]
pub enum CommentParseError {
	#[error("not a shrink-ray comment")]
	NotShrinkRay,
	#[error("not a valid version string: {}", .0)]
	NotVersion(#[from] semver::Error)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Comment {
	pub version: Version
}

impl Default for Comment {
	fn default() -> Self {
		let version = Version::from_str(env!("CARGO_PKG_VERSION")).unwrap();
		Comment { version }
	}
}

const PREFIX: &'static str = "shrink-ray/";

impl FromStr for Comment {
  type Err = CommentParseError;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    if s.len() < PREFIX.len() || !s[..PREFIX.len()].eq_ignore_ascii_case(PREFIX) {
    	return Err(CommentParseError::NotShrinkRay)
    }

    let version = Version::from_str(&s[PREFIX.len()..])?;
    Ok(Comment { version })
  }
}

impl fmt::Display for Comment {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "{}{}", PREFIX, self.version)
	}
}
