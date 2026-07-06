//! ansidrama CLI: `encode` (frames → WebP) and `record` (drive an embedded terminal → WebP).

use std::path::PathBuf;

use anyhow::{bail, Result};

fn usage() -> ! {
    eprintln!(
        "ansidrama — terminal sessions → animated WebP\n\
         \n\
         USAGE:\n\
         \x20 ansidrama encode <config.toml> [-o out.webp] [--dump-png <dir>]\n\
         \x20 ansidrama record <config.toml> [-o out.webp] [--dump-png <dir>]\n\
         \n\
         encode  assemble a WebP from a list of ANSI snapshots and/or title cards.\n\
         record  drive a command in an embedded terminal per a scene script, capturing each frame.\n\
         \n\
         -o           output path (overrides `out` in the config)\n\
         --dump-png   also write each rendered frame as a PNG into <dir> (debug)\n"
    );
    std::process::exit(2)
}

struct Args {
    config: PathBuf,
    out: Option<PathBuf>,
    dump_png: Option<PathBuf>,
}

fn parse_rest(mut it: impl Iterator<Item = String>) -> Result<Args> {
    let config = match it.next() {
        Some(c) if !c.starts_with('-') => PathBuf::from(c),
        _ => bail!("missing <config.toml>"),
    };
    let mut out = None;
    let mut dump_png = None;
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--out" => {
                out = Some(PathBuf::from(
                    it.next()
                        .ok_or_else(|| anyhow::anyhow!("-o needs a path"))?,
                ))
            }
            "--dump-png" => {
                dump_png = Some(PathBuf::from(
                    it.next()
                        .ok_or_else(|| anyhow::anyhow!("--dump-png needs a dir"))?,
                ))
            }
            other => bail!("unexpected argument {other:?}"),
        }
    }
    Ok(Args {
        config,
        out,
        dump_png,
    })
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_default();
    let a = match cmd.as_str() {
        "encode" | "record" => parse_rest(args)?,
        _ => usage(),
    };
    let out = a.out.as_deref();
    let dump = a.dump_png.as_deref();
    match cmd.as_str() {
        "encode" => ansidrama::encode(&a.config, out, dump),
        #[cfg(unix)]
        "record" => ansidrama::record(&a.config, out, dump),
        #[cfg(not(unix))]
        "record" => anyhow::bail!("record is only supported on unix platforms"),
        _ => unreachable!(),
    }
}
