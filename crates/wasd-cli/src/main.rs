//! `wasdc` — WASD Pascalコマンドラインコンパイラ。
//!
//! レキサ〜意味解析までの診断表示とAST確認（`check`/`parse`）に加え、
//! `compile --emit-pcode`でp-code（`wasd-pcode`）のテキスト表現
//! （逆アセンブリ的なニーモニック表示）を確認できる。ただし今回の
//! p-code生成は最小スコープ（`INTEGER`/`BOOLEAN`、制御構造、代入文のみ）
//! であり、実際にApple II p-System上で実行可能なバイナリを生成する
//! `build`のようなサブコマンドはまだ実装しない。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use wasd_driver::{
    compile, compile_to_pcode, locate, CompileOptions, Diagnostic, Dialect, Severity,
};

/// UCSD Pascal風のPascal処理系 `wasdc`。
#[derive(Parser, Debug)]
#[command(name = "wasdc", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// レキサ〜semaまで実行し、診断を表示するのみ。
    Check {
        file: PathBuf,
        /// 使用するdialect。デフォルトはISO 7185準拠の標準Pascal。
        #[arg(long = "std", value_enum, default_value_t = DialectArg::Iso7185)]
        std: DialectArg,
    },
    /// レキサ〜semaまで実行し、診断とASTを表示する。
    Parse {
        file: PathBuf,
        /// 使用するdialect。デフォルトはISO 7185準拠の標準Pascal。
        #[arg(long = "std", value_enum, default_value_t = DialectArg::Iso7185)]
        std: DialectArg,
        /// パース結果のASTをデバッグ出力する。
        #[arg(long)]
        emit_ast: bool,
    },
    /// レキサ〜semaまで実行し、成功すればp-codeを生成する。
    Compile {
        file: PathBuf,
        /// 使用するdialect。デフォルトはISO 7185準拠の標準Pascal。
        #[arg(long = "std", value_enum, default_value_t = DialectArg::Iso7185)]
        std: DialectArg,
        /// 生成したp-codeのテキスト表現（逆アセンブリ的なニーモニック
        /// 表示）を標準出力へ表示する。
        #[arg(long)]
        emit_pcode: bool,
    },
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum DialectArg {
    Iso7185,
    Ucsd,
}

impl From<DialectArg> for Dialect {
    fn from(value: DialectArg) -> Self {
        match value {
            DialectArg::Iso7185 => Dialect::Iso7185,
            DialectArg::Ucsd => Dialect::Ucsd,
        }
    }
}

/// I/Oエラー用の終了コード。ファイルが存在しない等、コンパイル以前の
/// エラーを診断（終了コード1）と区別するために使う。
const EXIT_IO_ERROR: u8 = 2;

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Command::Check { file, std } => run_check(&file, std.into()),
        Command::Parse {
            file,
            std,
            emit_ast,
        } => run_parse(&file, std.into(), emit_ast),
        Command::Compile {
            file,
            std,
            emit_pcode,
        } => run_compile(&file, std.into(), emit_pcode),
    }
}

fn read_source(path: &Path) -> Result<String, ExitCode> {
    std::fs::read_to_string(path).map_err(|err| {
        eprintln!("error: failed to read '{}': {err}", path.display());
        ExitCode::from(EXIT_IO_ERROR)
    })
}

fn run_check(file: &Path, dialect: Dialect) -> ExitCode {
    let source = match read_source(file) {
        Ok(source) => source,
        Err(code) => return code,
    };

    let options = CompileOptions::new(dialect).with_source_path(file.to_path_buf());
    let result = compile(&source, &options);

    print_diagnostics(&source, file, &result.diagnostics);
    exit_code_for(&result.diagnostics)
}

fn run_parse(file: &Path, dialect: Dialect, emit_ast: bool) -> ExitCode {
    let source = match read_source(file) {
        Ok(source) => source,
        Err(code) => return code,
    };

    let options = CompileOptions::new(dialect).with_source_path(file.to_path_buf());
    let result = compile(&source, &options);

    print_diagnostics(&source, file, &result.diagnostics);

    if emit_ast {
        if let Some(program) = &result.program {
            println!("{program:#?}");
        } else if let Some(unit) = &result.unit {
            println!("{unit:#?}");
        } else {
            println!("<no AST: parsing failed completely>");
        }
    }

    exit_code_for(&result.diagnostics)
}

fn run_compile(file: &Path, dialect: Dialect, emit_pcode: bool) -> ExitCode {
    let source = match read_source(file) {
        Ok(source) => source,
        Err(code) => return code,
    };

    let options = CompileOptions::new(dialect).with_source_path(file.to_path_buf());
    let result = compile_to_pcode(&source, &options);

    print_diagnostics(&source, file, &result.diagnostics);

    if emit_pcode {
        if let Some(pcode) = &result.pcode {
            print!("{pcode}");
        } else {
            println!("<no p-code: compilation failed or produced no PROGRAM to generate from>");
        }
    }

    exit_code_for(&result.diagnostics)
}

/// 診断を`rustc`風の体裁で標準出力へ表示する。1件もなければ簡潔な
/// 成功メッセージを表示する。
fn print_diagnostics(source: &str, file: &Path, diagnostics: &[Diagnostic]) {
    if diagnostics.is_empty() {
        println!("OK: no errors found");
        return;
    }

    for diag in diagnostics {
        let loc = locate(source, diag.span);
        let severity = match diag.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
            Severity::Hint => "hint",
        };

        println!("{severity}: {}", diag.message);
        println!(
            "  --> {}:{}:{}",
            file.display(),
            loc.start.line,
            loc.start.column
        );

        let line_num = loc.start.line.to_string();
        let gutter = " ".repeat(line_num.len());
        println!("{gutter} |");
        println!("{line_num} | {}", loc.line_text);

        let caret_indent = " ".repeat(loc.start.column.saturating_sub(1));
        let caret_len = if loc.end.line == loc.start.line {
            loc.end.column.saturating_sub(loc.start.column).max(1)
        } else {
            loc.line_text
                .len()
                .saturating_sub(loc.start.column.saturating_sub(1))
                .max(1)
        };
        let carets = "^".repeat(caret_len);
        println!("{gutter} | {caret_indent}{carets}");
        println!();
    }
}

fn exit_code_for(diagnostics: &[Diagnostic]) -> ExitCode {
    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        ExitCode::from(1)
    } else {
        ExitCode::from(0)
    }
}
