use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::error;

use crate::context::Context;

pub async fn get_comment(context: &mut Context, path: impl AsRef<Path>) -> Result<Option<String>, crate::Error> {
	let path = path.as_ref();
    let mut gm = context.command("ffprobe")?;
	gm
		.args(["-hide_banner"])
		.arg(path);

	let output = gm.output().await?;
	if !output.status.success() {
		return Err(crate::Error::Invocation("ffprobe", output.status))
	}

	let output = String::from_utf8_lossy(output.stderr.as_ref());
	let Some(index) = output.find("COMMENT") else {
		return Ok(None)
	};

	let Some((_, comment)) = output[index..].lines().next().map(str::trim).and_then(|i| i.split_once(':')) else {
		return Ok(None)
	};

	Ok(Some(comment.into()))
}

pub async fn convert(context: &mut Context, input: impl AsRef<Path>) -> Result<PathBuf, crate::Error> {
	let input = input.as_ref();
	let output = context.get_output_file(input, ".webm").await?;
	let log_file = context.get_output_file(input, "").await?;
	let metadata = format!("comment={}", context.get_comment());

	let mut ffmpeg = context.command("ffmpeg")?;
	ffmpeg.args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
		.arg(input)
		.args(["-c:v", "vp9", "-an", "-sn", "-strict", "-2", "-row-mt", "1", "-pass", "1", "-passlogfile"])
		.arg(&log_file)
		.args(["-f", "null", "-"]);

	if let Err(x) = context.run(ffmpeg, input).await {
		let log_file = full_log_file_name(log_file);
		if log_file.exists() {
			if let Err(x) = fs::remove_file(&log_file).await {
				error!("failed to delete pass log file `{}`: {}", log_file.display(), x);
			}
		}

		return Err(x)
	}

	let mut ffmpeg = context.command("ffmpeg")?;
	ffmpeg.args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
		.arg(input)
		.args(["-c:v", "vp9", "-c:a", "opus", "-strict", "-2", "-row-mt", "1", "-map_metadata", "-1", "-metadata"])
		.arg(metadata)
		.args(["-pass", "2", "-passlogfile"])
		.arg(&log_file)
		.args(["-f", "webm"])
		.arg(&output);

	let result = context.run(ffmpeg, input).await;
	let log_file = full_log_file_name(log_file);
	if let Err(x) = fs::remove_file(&log_file).await {
		error!("failed to delete pass log file `{}`: {}", log_file.display(), x);
	}

	match result {
		Ok(_) => Ok(output),
		Err(x) => {
			if output.exists() {
				if let Err(x) = fs::remove_file(&output).await {
					error!("failed to delete output file `{}`: {}", output.display(), x);
				}
			}

			Err(x)
		}
	}
}

fn full_log_file_name(path: PathBuf) -> PathBuf {
	let mut path = path.into_os_string();
	path.push("-0.log");
	PathBuf::from(path)
}
