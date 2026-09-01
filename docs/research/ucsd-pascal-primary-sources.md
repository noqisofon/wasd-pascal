# UCSD Pascal 一次資料調査メモ（UNCONFIRMED解消用）

`wasd-ast`/`wasd-sema`/`wasd-parser`に残っていた`UNCONFIRMED`コメントについて、
一次資料をあたった結果をまとめる。以前のセッションではサンドボックスの
ネットワークアクセスが制限されており、一次資料に直接あたれなかったため
`UNCONFIRMED`のまま実装されていた。

## 参照した資料

1. SofTech Microsystems, *UCSD p-System and UCSD Pascal Version IV: Internal
   Architecture Guide* (First edition, March 1981)
   https://archive.org/details/UCSD_P-System_UCSD_PASCAL_Internal_Architecture_Guide
   — p-machine/セグメント構造の内部仕様。UNIT/INTERFACE/IMPLEMENTATIONの
   Pascal言語構文そのものは薄い。
2. *UCSD PASCAL I.5 Manual* (Version I.5, September 1978)
   https://archive.org/details/bitsavers_univOfCalS5ManualSep78_10150393
   — Section 2.2.21「UNITS」に言語構文の解説があるはずだが、OCR化されたテキストが
   激しく文字化けしており、該当ページ本文を今回抽出できなかった（目次のページ番号は
   確認できた: p.156付近）。
3. Wikibooks, "Pascal Programming/Units"
   https://en.wikibooks.org/wiki/Pascal_Programming/Units
   — UCSD Pascalのunit機構の一般的な解説（一次資料ではないが基本構造の確認に有用）。

## 確認できたこと

### UNIT構文の基本構造
- `INTERFACE`部には**実装（本体）を書いてはならない**。宣言・シグネチャのみ。
  → `wasd-ast`の`InterfaceSection`（シグネチャのみ保持）設計は妥当。
- `IMPLEMENTATION`部に実際のprocedure/functionの本体を書く。
- UNITは単体では実行できないが、それ以外はプログラムと類似した構造を持つ
  （定数・型・変数・ルーチンを定義できる）。
- `USES`節はコンパイラに対し、指定したUNITのコードを取り込み、そのUNITの
  `INTERFACE`部で宣言された識別子を（あたかも自分のモジュールの一部であるかのように）
  利用可能にするよう指示する。

### UNITの初期化・終了処理（p-machine内部仕様、Internal Architecture Guideより確認）
- UNITはコンパイル単位ごとに「セグメント参照リスト」を持ち、名前`'***'`の特別な
  セグメント参照を通じて初期化・終了コードセクションが実行される。
- ホストプログラムを実行する前に、オペレーティングシステムは使用中の全UNITの
  リストを構築し、そのリストを使ってホストプログラムの呼び出し前後に
  各UNITの初期化・終了セクションを実行する。
- **これは`wasd-ast`の`Unit`構造体に欠けている概念**。UCSD PascalのUNITは
  Pascal言語レベルで見ると、`IMPLEMENTATION`部の末尾に
  `BEGIN ... END.`という初期化文（プログラム本体のような部分）を持つ場合がある
  （一般的なUCSD Pascal実装の慣用）。次のAST拡張で
  `ImplementationSection`に`init_body: Option<Block>`のようなフィールドを
  追加することを検討する価値がある（今回は追加していない。コメントでの
  申し送りのみ）。

### コンパイラディレクティブ
- `$R2`/`$R4`（`realsize`を32/64bitに設定）はInternal Architecture Guideで
  存在が確認できた。`$I`（include）は慣用的に知られる既知のディレクティブ。
  → `wasd-sema::typeck::check_compiler_directive`の既知ディレクティブ判定に
  `$R2`/`$R4`を追加した（レキサーは`$`直後の英数字を貪欲に`name`として取る
  ため、`name`は`"R"`ではなく`"R2"`/`"R4"`そのものになる点に注意）。

## 未確認のまま残った事項（要継続調査）

1. **`IMPLEMENTATION`部の初期化文の正確な構文** — 上記の通り存在は内部アーキテクチャ
   仕様から示唆されるが、Pascal言語レベルでの正確な構文（`BEGIN...END.`が必須か、
   省略可能か）はUsers' Manual本文の該当ページ(I.5 Manual 該当章、または
   Version IV.0 Users' Manual)を読む必要がある。OCRの文字化けにより今回未確認。

2. **`IMPLEMENTATION`部限定の非公開procedure/function** — `INTERFACE`部に
   現れない補助的なルーチンを`IMPLEMENTATION`部だけに書けるかどうか。
   一般的なUCSD Pascal実装解説（Wikibooks等）では触れられているが、
   一次資料での明記は未確認。

3. **UNIT間の循環参照の可否** — 明記した一次資料は見つからず。

4. **STRING[n]の既定長（角括弧省略時）** — 今回は未調査。別途確認が必要。

5. **コンパイラディレクティブの種類** — `$R2`/`$R4`（realsizeを32/64bitに設定、
   Internal Architecture Guideで確認済み）と`$I`（include、慣用的に知られる）
   以外の一覧は未確認（`$U`/`$S`など）。

## 反映済みの変更

- `crates/wasd-ast/src/decl.rs`: `Unit`/`ImplementationSection`のドキュメントを
  `CONFIRMED`/`UNCONFIRMED`に整理し、UNIT初期化・終了処理（`'***'`セグメント
  参照）の存在を申し送り事項として明記。
- `crates/wasd-parser/src/parser.rs`: `parse_unit`/`parse_string_n_type`の
  コメントを更新（ネットワーク遮断が理由ではなく、資料自体に未確認事項が残る
  旨に修正）。
- `crates/wasd-sema/src/typeck.rs`: `check_compiler_directive`で`$R`を既知の
  ディレクティブとして受理するよう変更し、対応するテストを追加。

残る未確認事項（上記5点）は`UNCONFIRMED`のまま維持し、追加調査が必要な旨を
コメントに明記している。
