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
（レキサ・パーサー・意味解析）と、LSP サーバー (`wasd-lsp`) の
診断表示 (`textDocument/publishDiagnostics`) まで。ホバー・補完・
定義へジャンプ等は次段階。p-machine の命令セット実装そのものは
別リポジトリ／別 workspace を想定している。

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

## `wasd-lsp` の使い方

`wasd-lsp`は、標準入出力（stdio）経由でLSPクライアントに接続できる
Language Serverバイナリ。今回のスコープは診断表示
（`textDocument/publishDiagnostics`）のみで、ホバー・補完・定義へ
ジャンプ等は未実装。dialectは現時点ではサーバー起動時に固定
（デフォルトのISO 7185）で、エディタ設定からの切り替えには未対応。

```sh
# ビルドする。
cargo build -p wasd-lsp

# バイナリは target/debug/wasd-lsp (または --release なら target/release/wasd-lsp) に生成される。
cargo run -p wasd-lsp
```

サーバーはstdin/stdoutでJSON-RPCのLSPメッセージを待ち受ける（他の
出力は行わない）ので、単体で起動しても何も表示されずブロックする
のが正常な動作。エディタ側から接続するには、汎用のLSPクライアント
拡張（VS Codeなら例えば`vscode-languageclient`を使った簡易拡張）に
サーバーコマンドとして上記のバイナリパスを指定し、`.pas`ファイルを
対象言語として登録すればよい。本リポジトリでは今回、専用のVS Code
拡張の作成はスコープ外としている。

`textDocument/didOpen`/`didChange`（`TextDocumentSyncKind::FULL`、
つまり変更のたびに全文が送られてくる）を受けて`wasd-driver::compile`
を呼び出し、返ってきた診断を`publishDiagnostics`で配信する。
ソース上のバイト位置（`Span`）からLSPが期待する「0始まりの行番号 +
UTF-16コードユニット単位の列」への変換ロジックは`src/position.rs`に
あり、日本語コメント等マルチバイト文字を含むソースについてもユニット
テストで検証している。

## 次のステップ

- `wasd-lsp`: ホバー・補完・定義へジャンプ、dialectのエディタ設定連携
- `PROCEDURE`/`FUNCTION`・配列・レコード・ポインタ・UNITのp-code生成
- p-machine 命令セットの実装（別リポジトリ or 別 workspace 想定）
