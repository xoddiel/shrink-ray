use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use rand::Rng;

const ALPHABET: &str = "abcdefghijklmnopqrstuvwxyz0123456789";
const LENGTH: usize = 8;

pub fn file(path: impl AsRef<Path>, suffix: Option<&OsStr>) -> PathBuf {
	let path = path.as_ref();
	let mut rng = rand::thread_rng();
	let mut prefix = path.parent().unwrap().join(path.file_stem().unwrap()).into_os_string();
	prefix.push("-");
	loop {
		let mut buf = prefix.clone();

		for _ in 0..LENGTH {
			let index = rng.gen_range(0..ALPHABET.len());
			buf.push(&ALPHABET[index..=index]);
		}

		if let Some(suffix) = suffix {
			buf.push(suffix);
		}

		let path = PathBuf::from(buf);
		if !path.exists() {
			return path;
		}
	}
}
