use std::{ffi::OsString, path::{Path, PathBuf}};
use tokio::fs;
use tracing::{error, trace};

use crate::{comment::Comment, context::Context};

pub async fn get_comment(context: &mut Context, path: impl AsRef<Path>) -> Result<Option<Comment>, crate::Error> {
	let path = path.as_ref();
    let mut gm = context.command("gm")?;
	gm
		.args(["identify", "-verbose"])
		.arg(path);

	let output = gm.output().await?;
	if !output.status.success() {
		return Err(crate::Error::Invocation("gm", output.status))
	}

	let output = String::from_utf8_lossy(output.stdout.as_ref());
	let Some(index) = output.find("Comment:") else {
		return Ok(None)
	};

	let Some(comment) = output[index..].lines().next().map(str::trim) else {
		return Ok(None);
	};

	comment.parse().map(Some).map_err(crate::Error::from)
}

pub async fn convert(context: &mut Context, comment: Comment, input: impl AsRef<Path>) -> Result<PathBuf, crate::Error> {
	let input = input.as_ref();
	let output = context.get_output_file(input, ".jpg").await?;
	let comment = comment.to_string();

	let mut output_arg = OsString::from("jpeg:");
	output_arg.push(&output);

	let mut gm = context.command("gm")?;
	gm
		.arg("convert")
		.arg(input)
		.arg("-strip")
		.arg("-comment")
		.arg(comment)
		.arg(output_arg);

	match context.run(gm, input).await {
		Ok(_) => Ok(output),
		Err(x) => {
			if output.exists() {
				trace!("error raised; deleting output file `{}`...", output.display());
				if let Err(x) = fs::remove_file(&output).await {
					error!("failed to delete output file `{}`: {}", output.display(), x);
				}
			}

			Err(x)
		}
	}
}
