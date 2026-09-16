# Epub3-Kindle

[![crates.io](https://img.shields.io/crates/v/epub3-kindle.svg)](https://crates.io/crates/epub3-kindle)

[English](README.md) | [日本語](README.ja.md)

[EPUB](https://www.w3.org/TR/epub-33/)を、AZW3またはMOBIへ変換するRustライブラリおよびCLIです。

EPUBのレイアウト、ルビ、ナビゲーション、埋め込みフォント、CSSなどに対応し、特に日本語縦書きを重視しています。

あわせて、[EPUB 3.3](https://www.w3.org/TR/epub-33/)からの変換について、対応範囲を監査・検証しています。

## ダウンロード

最新版は[Releases](https://github.com/cbz-tools/Epub3-Kindle/releases/latest)からダウンロードできます。

| プラットフォーム | パッケージ |
|---|---|
| Windows x64 | `epub3-kindle-vX.Y.Z-windows-x64.zip` |
| Linux x64 | `epub3-kindle-vX.Y.Z-linux-x64.tar.gz` |
| macOS Apple Silicon | `epub3-kindle-vX.Y.Z-macos-arm64.tar.gz` |

アーカイブを展開し、`epub3-kindle` を直接実行してください。

### Cargoでインストール

Rust 1.85以降が必要です。

```bash
cargo install epub3-kindle
```

## クイックスタート

EPUBを変換します。

```bash
epub3-kindle input.epub
```

出力先を省略した場合、入力パスの拡張子を `.mobi` に置き換えたパスが使用されます。

## 主な機能

- AZW3出力
- MOBI出力
- 日本語縦書き
- 右から左へのページ進行
- ルビ・傍点
- Kindle向けナビゲーション・読書順序
- 埋め込みフォント
- Kindle向けCSS変換
- PalmDOC圧縮
- カバーリソース・書籍内カバー表示
- 対応範囲内でのEPUB 3.3互換性
- CLIに加えてRustライブラリAPIを提供

## スクリーンショット

以下は、同じ汎用EPUB 3ドキュメント用サンプルをKindle実機で表示した比較です。

### Epub3-Kindle

| 表紙 | 目次 | 縦書き本文 |
|---|---|---|
| [![表紙](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/Epub3-Kindle-01.JPEG)](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/Epub3-Kindle-01.JPEG) | [![目次](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/Epub3-Kindle-02.JPEG)](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/Epub3-Kindle-02.JPEG) | [![縦書き本文](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/Epub3-Kindle-03.JPEG)](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/Epub3-Kindle-03.JPEG) |

### KindleGen

| 表紙 | 目次 | 縦書き本文 |
|---|---|---|
| [![表紙](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/kindlegen-01.JPEG)](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/kindlegen-01.JPEG) | [![目次](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/kindlegen-02.JPEG)](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/kindlegen-02.JPEG) | [![縦書き本文](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/kindlegen-03.JPEG)](https://raw.githubusercontent.com/cbz-tools/Epub3-Kindle/main/docs/assets/kindlegen-03.JPEG) |

この比較では、読者から見える差は確認できませんでした。

## 目的

KindleGenは長年Amazonから入手できない状態が続いており、新しいツールから利用する依存先としては現実的ではなくなっています。

Epub3-Kindleは現行世代のKindleを対象としています。Kindle Touch (4th Generation) より前の端末をサポート対象外としているため、`.mobi` に古いKindle向けの完全な本文をもう一式含める必要がありません。

その代わり、`.mobi` として認識するための小さな互換セクションと、実際に読むための新しいKindle向け本文を格納します。

| 出力 | 内部構成のイメージ |
|---|---|
| AZW3 | 新しいKindle向け本文のみ |
| Epub3-Kindle MOBI | 最小互換部分 + 新しいKindle向け本文 |
| KindleGen MOBI | 古いKindle向け本文 + 新しいKindle向け本文 |

内部的には、最小互換部分がKF7、実際の読書本文がKF8です。

KindleGenとの互換性は、対応範囲内で同じように読めることを目標としています。すべてのEPUBやKindleGenのすべての動作を保証するものではありません。

## KindleGenの置き換え

Epub3-Kindleは、既存のKindleGenベースのワークフローを置き換えやすいように設計しています。

`epub3-kindle.exe` を `kindlegen.exe` にリネームし、既存の `kindlegen.exe` と差し替えることで、KindleGenを呼び出している既存のツールやスクリプトから利用できます。

ただし、Epub3-KindleはKindleGenの完全な代替実装ではありません。対応する入力、オプション、出力形式は、このREADMEおよび監査で定義した範囲に限定されます。

## KindleGenとの比較

| | KindleGen | epub3-kindle |
|---|---|---|
| 入力 | EPUB、HTML/OPF、その他の対応入力 | EPUB |
| 出力 | 従来形式 + KF8 Kindle形式 | AZW3（KF8） または MOBI（最小KF7互換 + KF8） |
| 日本語縦書き / ルビ | 対応 | 対応、Kindle実機で検証済み |
| 書籍ビューアー内のカバー表示 | 対応 | 対応、Kindle実機で検証済み |
| Kindleライブラリのカバーサムネイル | 対応 | MOBI: 対応 / AZW3: 未対応 |
| [Kindle Touch (4th Generation) より前の端末](https://digprjsurvey.amazon.com/csad/help/node/GK33S847NN4V6Y83) | サポート対象 | サポート対象外 |
| 入手性 | Amazonからの配布終了 | オープンソースで継続的にメンテナンス |

## パフォーマンス

以下は、実データを用いた測定例の一つです。

入力準備：

| 段階 | サイズ |
|---|---:|
| 元TXT（テキストのみ、画像なし） | 49.58 MiB (51,992,309 bytes) |
| AozoraEpub3で生成したEPUB | 17.92 MiB (18,790,298 bytes) |

3つの変換すべてに、同じ生成済みEPUBを入力しました。

| | KindleGen MOBI | Epub3-Kindle MOBI | Epub3-Kindle AZW3 |
|---|---:|---:|---:|
| 変換時間 | 90.173 s | 2.754 s | 2.752 s |
| 出力サイズ | 83.71 MiB (87,771,662 bytes) | 47.41 MiB (49,710,628 bytes) | 47.40 MiB (49,701,730 bytes) |

各変換を同じEPUBに対して5回実行し、最速の1回と最遅の1回だけを除外した残り3回の算術平均を変換時間として記載しています。

この測定では、Epub3-Kindle MOBIはKindleGen MOBIより約32.7倍高速で43.36%小さく、Epub3-Kindle AZW3は約32.8倍高速で43.37%小さくなりました。

Epub3-KindleのMOBIとAZW3は、どちらもデフォルトのPalmDOC圧縮で実行しています。

結果は入力、ツールのバージョン、ハードウェアによって異なります。TXTからEPUBを生成する時間は変換時間に含めていません。

## CLI

```text
epub3-kindle <input.epub> [-o <output.azw3|output.mobi>] [-c0 | -c1]
    [-verbose] [-dont_append_source] [-donotaddsource]
```

`-c1` がデフォルトで、PalmDOC圧縮を使用します。`-c0` はテキストレコードを非圧縮で格納します。`-dont_append_source` と `-donotaddsource` はKindleGen互換の無処理オプションとして受け付けます。このコンバーターは元のEPUBを埋め込みません。`-c2`（HUFF/CDIC）および無関係なKindleGenオプションは意図的にサポートしていません。`-o` を省略すると、入力ファイルの拡張子を `.mobi` に置き換えます。明示的な `.mobi` では最小KF7互換セクションとKF8本文からなるMOBIを、明示的な `.azw3` ではAZW3を生成します。

## ライブラリ

```rust
use epub3_kindle::{convert_bytes, Compression, ConvertOptions};

let azw3 = convert_bytes(
    &epub_bytes,
    &ConvertOptions { compression: Compression::PalmDoc },
)?;
```

このcrateは `convert_file(input, output, options)` も提供します。ファイルAPIは出力先の拡張子で形式を選択し、`.azw3`ではAZW3、`.mobi`では最小KF7互換セクションとKF8本文からなるMOBIを生成します。シリアライズが成功した後にのみ、同じディレクトリの一時ファイルから出力先を置き換えます。

`convert_file`では、変換結果を同じディレクトリの一時ファイルへシリアライズし、成功後に出力先をアトミックに置き換えます。グローバルな可変変換状態、共有一時ファイル名、カレントディレクトリの変更、内部並列ランタイムは使用しません。独立した変換は安全に並行実行できます。通常のファイル書き込みと同様、完全に同じ出力パスへ複数の変換を同時実行する場合の調整は呼び出し側の責任です。

## 制限事項

Kindle Touch (4th Generation) より前のKindle端末はサポートしていません。`.azw3`ではAZW3を、`.mobi`では最小KF7互換セクションとKF8本文からなるMOBIを生成します。`.mobi`の互換セクションは完全な旧MOBI7本文ではありません。

書籍内のカバー表示には対応しています。Kindleライブラリのカバーサムネイルは、MOBIでは対応、AZW3では未対応です。

## 検証

検証には、汎用・合成EPUBフィクスチャ、外部生成元との相互運用性ソース、同一入力に対するKindleGen比較、独立した解析、Kindle実機での確認を含む、より広い互換性コーパスを用いています。

EPUBからの変換について対応範囲を監査しています。これはEPUB 3.3全体やKindleGenの全エッジケースを網羅するという主張ではありません。

EPUBについては、[EPUB 3.3仕様](https://www.w3.org/TR/epub-33/)を基準にしています。

Kindle固有の変換および互換性については、[Amazon Kindle Publishing Guidelines](https://kindlegen.s3.amazonaws.com/AmazonKindlePublishingGuidelines.pdf)、同一入力に対するKindleGen出力、およびKindle実機を参照・検証しています。

読書順序、ナビゲーション、テキストの忠実性、日本語縦書き、ルビなど、読者から見える挙動についてもKindle実機で確認しています。

対応範囲、検証方法、互換性の境界、E2Eおよび実機での検証結果の詳細については、[EPUB → Kindle Conversion Audit](docs/CONVERSION_AUDIT.md)を参照してください。

この監査は、本コンバーターで検証された動作についての正式な技術記録です。

## 謝辞

このプロジェクトのKF8/MOBI形式の解析および実装では、以下のオープンソースプロジェクトおよび技術資料から多大な恩恵を受けました。

- [MobileRead Wiki — MOBI](https://wiki.mobileread.com/wiki/MOBI) — PalmDOC、MOBI/EXTHヘッダー、インデックス、レコード構造などを理解するうえで重要な資料となりました。
- [Kindling](https://github.com/ciscoriordan/kindling) — Rust製Kindleツールキットであり、KF8/MOBI構造を理解するうえで有用な参考実装となりました。
- [calibre](https://github.com/kovidgoyal/calibre) — Kindle/MOBI実装は、フォーマットの挙動や実装詳細を理解するうえで非常に有用な参考資料となりました。
- [KindleUnpack](https://github.com/kevinhendricks/KindleUnpack) — KindleGenが生成したファイルの調査・解析に広く使用しました。

これらの成果と情報を公開してくださった作者、コントリビューター、およびコミュニティの皆様に感謝します。

このプロジェクトは独立した実装であり、Amazon、MobileRead、Kindling、calibre、KindleUnpackとは関係ありません。

## ライセンス

MIT Licenseの下でライセンスされています。

詳細は[LICENSE](LICENSE)を参照してください。

サードパーティ依存関係の概要は[THIRDPARTY_LICENSES.md](THIRDPARTY_LICENSES.md)を参照してください。

## 変更履歴

リリース履歴は[CHANGELOG.md](CHANGELOG.md)を参照してください。