//! Inspect installation media without mounting them.

mod detect;
mod image;
mod iso9660;
mod rules;
mod udf;
mod wim;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Result, bail};

use crate::image::{FileSystem, Image};
use crate::iso9660::Iso9660;
use crate::rules::Rules;
use crate::udf::Udf;

const USAGE: &str = "\
Usage: pxvirt-isoinfo [OPTIONS] IMAGE

Detect the operating system on an installation image and print it as JSON.

Options:
  --rules FILE   use the given rules file
  --pretty       pretty print the JSON output
  --ls PATH      list a directory of the image instead
  --dump-rules   print the built-in rules
  -h, --help     show this help
  -V, --version  show the version
";

struct Args {
    rules: Option<PathBuf>,
    pretty: bool,
    ls: Option<String>,
    image: PathBuf,
}

fn parse_args() -> Result<Option<Args>> {
    let mut rules = None;
    let mut pretty = false;
    let mut ls = None;
    let mut image = None;

    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("-h" | "--help") => {
                print!("{USAGE}");
                return Ok(None);
            }
            Some("-V" | "--version") => {
                println!("pxvirt-isoinfo {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            Some("--dump-rules") => {
                print!("{}", rules::BUILTIN);
                return Ok(None);
            }
            Some("--pretty") => pretty = true,
            Some("--rules") => match args.next() {
                Some(path) => rules = Some(PathBuf::from(path)),
                None => bail!("--rules requires an argument"),
            },
            Some("--ls") => match args.next().and_then(|p| p.into_string().ok()) {
                Some(path) => ls = Some(path),
                None => bail!("--ls requires an argument"),
            },
            Some(opt) if opt.starts_with('-') && opt.len() > 1 => bail!("unknown option '{opt}'"),
            _ if image.is_none() => image = Some(PathBuf::from(arg)),
            _ => bail!("too many arguments"),
        }
    }

    let Some(image) = image else {
        bail!("no image given\n\n{USAGE}");
    };

    Ok(Some(Args { rules, pretty, ls, image }))
}

/// Open the file system, UDF is preferred as it holds the real content on UDF bridge media.
fn open_fs(image: &Image) -> Result<(Box<dyn FileSystem>, &'static str)> {
    if udf::detect(image) {
        match Udf::open(image) {
            Ok(fs) => return Ok((Box::new(fs), "udf")),
            Err(err) => {
                if let Some(fs) = Iso9660::open(image)? {
                    eprintln!("warning: {err:#}, falling back to ISO 9660");
                    return Ok((Box::new(fs), "iso9660"));
                }
                return Err(err);
            }
        }
    }
    match Iso9660::open(image)? {
        Some(fs) => Ok((Box::new(fs), "iso9660")),
        None => bail!("no ISO 9660 or UDF file system found"),
    }
}

fn list(image: &Image, fs: &dyn FileSystem, path: &str) -> Result<()> {
    let Some(dir) = fs.lookup(image, path)? else {
        bail!("'{path}' not found");
    };
    let entries = if dir.is_dir { fs.list(image, &dir)? } else { vec![dir] };
    for entry in entries {
        let kind = if entry.is_dir { 'd' } else { '-' };
        println!("{kind} {:>14} {}", entry.size, entry.name);
    }
    Ok(())
}

fn run() -> Result<()> {
    let Some(args) = parse_args()? else {
        return Ok(());
    };

    let image = Image::open(Path::new(&args.image))?;
    let (fs, fs_name) = open_fs(&image)?;

    if let Some(path) = &args.ls {
        return list(&image, fs.as_ref(), path);
    }

    let rules = Rules::load(args.rules.as_deref())?;
    let info = detect::Detector::new(&image, fs.as_ref(), &rules).detect(fs_name)?;

    let out = if args.pretty { serde_json::to_string_pretty(&info)? } else { serde_json::to_string(&info)? };
    println!("{out}");

    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("pxvirt-isoinfo: {err:#}");
            ExitCode::FAILURE
        }
    }
}
