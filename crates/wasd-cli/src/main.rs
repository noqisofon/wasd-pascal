//! `wasdc` — WASD Pascalコマンドラインコンパイラ（プレースホルダ）。

use clap::Parser;
use wasd_driver::Dialect;

/// UCSD Pascal風のPascal処理系 `wasdc`。
#[derive(Parser, Debug)]
#[command(name = "wasdc", version, about)]
struct Cli {
    /// コンパイル対象のソースファイル。
    input: Option<String>,

    /// 使用するdialect。デフォルトはISO 7185準拠の標準Pascal。
    #[arg(long = "std", value_enum, default_value_t = StdArg::Iso7185)]
    std: StdArg,
}

#[derive(Copy, Clone, Debug, clap::ValueEnum)]
enum StdArg {
    Iso7185,
    Ucsd,
}

impl From<StdArg> for Dialect {
    fn from(value: StdArg) -> Self {
        match value {
            StdArg::Iso7185 => Dialect::Iso7185,
            StdArg::Ucsd => Dialect::Ucsd,
        }
    }
}

fn main() {
    let cli = Cli::parse();
    let dialect: Dialect = cli.std.into();

    match cli.input {
        Some(path) => {
            eprintln!("wasdc: not yet implemented (input={path}, dialect={dialect:?})");
        }
        None => {
            eprintln!("wasdc: no input file (dialect={dialect:?})");
        }
    }
}
