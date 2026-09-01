# WASD Pascal

UCSD Pascal ライクな Pascal 処理系を Rust で実装するプロジェクト。

歴史的忠実性を重視し、SofTech Microsystems Internal Architecture Reference
Manual 等の一次資料に基づいて実装する。未確認の仕様を推測で埋めない方針。

## 最終ゴールと3層構造

このプロジェクトは、最終的に UCSD p-System スタック上で動く
Wizardry 風 3D ダンジョンゲームの実現を見据えている。全体は次の3層で構成される想定。

1. **WASD Pascal** (本リポジトリ) — UCSD Pascal 風の Pascal コンパイラ。
   ソースを p-code (UCSD p-System の中間表現) にコンパイルする。
2. **p-machine emulator** (別リポジトリ想定) — p-code を実行する
   p-machine (p-System 仮想機械) のエミュレータ。
3. **Wizardry 風ゲーム** — WASD Pascal でコンパイルし、p-machine emulator
   上で動作する 3D ダンジョン RPG。

## 現在のスコープ

このリポジトリの現段階のスコープは、コンパイラのフロントエンド
（レキサ・パーサー・意味解析）と LSP サーバーの土台まで。p-machine の
命令セット実装そのものは別リポジトリ／別 workspace を想定している。

## クレート構成

Cargo workspace として構成し、依存の向きは一方向（下位クレートが
上位クレートに依存されることはない）。

```
wasd-ast     ← 他クレートから依存されるのみ。依存先なし（AST/Span/Dialect/Diagnostic）
wasd-lexer   ← wasd-ast
wasd-parser  ← wasd-lexer, wasd-ast
wasd-sema    ← wasd-ast（意味解析・型検査・dialectチェック）
wasd-pcode   ← wasd-ast, wasd-sema（p-code (IR) 生成）
wasd-driver  ← 上記すべて（CLIとLSPが共有するコアAPI）
wasd-cli     ← wasd-driver（`wasdc` コマンド本体）
wasd-lsp     ← wasd-driver, tower-lsp（LSPサーバー）
```

## dialect 方針

デフォルトは ISO 7185 準拠の標準 Pascal (`Dialect::Iso7185`)。UCSD 拡張
（`UNIT`/`INTERFACE`/`IMPLEMENTATION`、`STRING[n]`、`OTHERWISE`、16進数
リテラル `$FF`、コンパイラディレクティブ `(*$I file*)` など）は
`--std=ucsd` でオプトイン有効化する（gcc の `-std=` に相当する発想）。

パーサーは dialect に関わらず単一で、UCSD 拡張構文も常にパース可能とする。
dialect 違反（「この構文は現在の dialect では使えない」）の検出・報告は
意味解析フェーズ（`wasd-sema`）が担当する。パーサーレベルでは拒否しない。

## ビルド方法

```sh
cargo build --workspace
cargo test --workspace
```

## `wasdc` の使い方

`wasd-cli`クレートが提供するコマンドラインコンパイラ`wasdc`で、`examples/`
以下のサンプルを実際にチェックできる。まだp-code生成（`wasd-pcode`）が
無いため、`check`/`parse`（構文・意味解析のチェックとAST確認）のみを提供する。

```sh
# レキサ〜semaまで実行し、診断を表示する。
cargo run -p wasd-cli -- check examples/hello.pas

# UCSD dialectを有効化しないとUNIT宣言はdialectエラーになる。
cargo run -p wasd-cli -- check examples/ucsd_unit.pas
cargo run -p wasd-cli -- check examples/ucsd_unit.pas --std=ucsd

# ASTをデバッグ出力する。
cargo run -p wasd-cli -- parse examples/procedures.pas --emit-ast
```

診断にエラー（`error:`）が1件でもあれば終了コードは`1`、警告のみ・
エラーなしであれば`0`、ファイルが存在しない等のI/Oエラーは`2`になる。

## 次のステップ

- `wasd-lexer`: UCSD Pascal の字句仕様の実装（一次資料に基づく）
- `wasd-ast`: 文/式/宣言の AST ノード定義
- `wasd-parser`: 再帰下降パーサーの実装
- p-machine 命令セットの実装（別リポジトリ or 別 workspace 想定）
