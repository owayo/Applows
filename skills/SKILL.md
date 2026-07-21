---
name: applows
description: >-
  Applows 言語 (.aplo) を書くためのリファレンス。シェル風の 1 ソースから、バニラ
  Windows 11 と macOS の両方で追加ランタイム不要で動く単一ポリグロット .bat を生成する
  コンパイラ (cross-platform / polyglot script)。Windows と macOS の両対応スクリプト・
  クロスプラットフォームのブートストラップ / プロビジョニング / セットアップスクリプトを
  書く、`.aplo` を書く/直す、「applows」でコンパイルする場面で発動。型付き・安全側の
  言語なので、書いたら必ず `applows check` で検証する。
allowed-tools: Bash(applows:*)
---

# Applows

Applows はシェル風の 1 ソース (`.aplo`) から、**バニラ Windows 11 と macOS の両方で
追加ランタイムなしにネイティブ実行できる単一のポリグロットスクリプト (`.bat`)** を生成する
コンパイラ。出力 1 ファイルが Windows Batch + PowerShell 5.1 + macOS `/bin/sh`(bash)+zsh
として同時に valid。用途はクロス OS のブートストラップ / プロビジョニング / CI セットアップ。

**Applows は型付きで安全側に倒した小さな言語**。暗黙の型変換や truthiness が無く、危険
パターンはコンパイルエラーで弾かれる。**書いたら必ず `applows check <file>.aplo` で検証**し、
エラーが出たら本書のルールに照らして直す。これが正しいコードを書く最短ループ。

## CLI

```bash
applows build  input.aplo [-o out.bat]        # 単一ポリグロット .bat を生成 (出力既定は .bat)
applows check  input.aplo                      # 型検査のみ (出力しない) — 書いたら必ずこれで検証
applows emit   input.aplo --target sh|powershell|ir   # 中間生成物を確認 (デバッグ用)
```
生成した `.bat` は macOS では `sh out.bat` / `./out.bat`、Windows では `out.bat` で実行。

## 最初に覚える鉄則 (AI が最も踏む間違い)

ここを外すと `check` が通らない。エラーメッセージも日本語で対応ルールを指す。

1. **文字列補間 `{...}` は変数名のみ**。関数結果・式は不可 → `let` で受ける。
   ```
   let d = script_dir()
   print "dir={d}"                 # OK   ("dir={script_dir()}" は不可)
   ```
2. **文字列連結に `+` は使えない** (`+` は Int 専用) → 補間で連結する。
   ```
   let full = "{first} {last}"     # OK   (first + " " + last は不可)
   ```
3. **外部コマンドは argv 配列**: `run(["git", "status"])`。文字列 1 本 `run("git status")` は不可。
   **戻り値は終了コード (Int) だけ。コマンドの stdout を文字列として受け取る方法は無い**
   (stdout は画面へそのまま流れる)。出力が要るならファイルへ書かせて `read_text` で読む。
4. **条件は真偽値だけ** (truthiness 無し)。成否は `== 0` で明示。`exists()` 等の Bool は
   条件専用で、**変数に代入できない** (`let x = exists(p)` は不可)。
   ```
   let code = run(["git", "--version"])
   if code == 0 { print "ok" }     # OK   (if run([...]) { } は不可)
   if not exists("cfg.txt") { print "no cfg" }
   ```
5. **`and` / `or` / `not` の内側に副作用呼び出し** (`run` / `http_download` / ユーザ関数) は
   書けない → 先に `let` で受けてから比較する。
   ```
   let c = run(["git", "fetch"])
   let mode = env("MODE", "on")
   if c == 0 and mode == "on" { print "go" }   # OK  (if run([...]) == 0 and ... は不可)
   ```
6. **`Text` と `Int` は混ぜられず、変換関数も無い**。`arg(i)` や `read_text` の結果 (Text) で
   算術はできない。件数判定は `argc()` (Int) を使う。
7. **`arg(i)` の i は整数リテラルのみ** (変数不可)。引数を順に処理するなら `for a in args()`。
8. **文字列の比較は `==` `!=` のみ** (大小比較 `< <= > >=` は Int 専用)。
9. **リストは変数へ代入・補間できない**。`run(...)` の argv と `for` の反復にだけ書ける。

## MVP に無いもの (書こうとしない)

以下は**構文ごと存在しない**。使うとエラーになるので、右の代替で設計する。

| 無いもの | 代替 |
|---|---|
| `break` / `continue` | ループ条件・フラグ変数で制御する (`while n > 0 and done == 0` 等) |
| コマンド出力のキャプチャ | 出力をファイルへ書かせて `read_text` で読む |
| 正規表現・`replace` | `upper` / `lower` / `trim` と `==` 比較で済む範囲に留める |
| 辞書・オブジェクト・リスト変数 | スカラ変数を並べる |
| OS 分岐構文 (if windows 等) | 組み込み関数だけで書く (下記「移植性」) |
| raw シェル/PowerShell 埋め込み | 組み込み関数 + `run` で表現する |
| 文字列⇔数値の変換 | 数値は Int リテラル・`argc()`・算術からのみ得る |
| 再帰・クロージャ・パイプ・リダイレクト | 逐次処理 + 終了コードで組む |

## 型

| 型 | 説明 |
|---|---|
| `Text` | 文字列 (UTF-8)。`"..."` |
| `Int` | 64bit 整数。`42`, `-3` |
| `Bool` | 条件の中だけに存在。変数へ代入・格納できない。 |
| `List` | リスト (要素は Text / Int)。argv と for 反復専用。 |

## 構文

```
# コメント (# から行末)

let name = "world"            # 変数の宣言/再代入
let count = 3

print "hello, {name}!"        # 出力 (改行付き)。補間は {変数名} のみ
let twice = count * 2
print "count={count} twice={twice}"

# エスケープ: \n \t \r \\ \" \{ \}   (リテラルの波括弧は \{ \})

# 算術 (Int): + - * / %  (/ は 0 方向への整数除算)
let total = count * 2 + 1

# 比較: == != (Text/Int) / < <= > >= (Int のみ) / 論理: and or not
if count > 2 and name == "world" {
  print "big & world"
} else if count == 0 {
  print "zero"
} else {
  print "other"
}

# while (break は無い。条件で抜ける)
let n = count
while n > 0 {
  print "n={n}"
  let n = n - 1
}

# for: 整数レンジ (両端含む) と リスト反復
for i in 1 to 3 {
  print "i={i}"
}
for fruit in ["apple", "banana"] {
  print "fruit={fruit}"
}

# 関数: トップレベルのみ / 値渡し / パラメータは Text 扱い / 戻り値は終了ステータス(Int)
# 再帰・前方参照・外側変数の参照は不可 (必要な値は引数で渡す)
fn greet(who) {
  print "hi {who}"
  return 0
}
greet("team")

exit 0                        # 終了コード (0-255)。`exit` 単独 = exit 0
```

## 組み込み関数

| 関数 | 戻り | 用途 |
|---|---|---|
| `print EXPR` (文) | — | 改行付き出力 |
| `run(list)` | Int (終了コード) | 外部コマンド実行 (argv 配列)。stdio は継承 (出力キャプチャ不可) |
| `env(name, default)` | Text | 環境変数。`name` は識別子リテラル (`"PATH"` 等) |
| `arg(i)` / `argc()` / `args()` | Text / Int / List | スクリプト引数。`i` は 1 始まりの整数リテラル。**トップレベルのみ** (関数内不可)。反復は `for a in args()` |
| `exists(p)` / `is_file(p)` / `is_dir(p)` | Bool | 存在判定 (条件でのみ使用可) |
| `read_text(p)` | Text | ファイル読み込み (UTF-8) |
| `write_text(p, s)` / `append_text(p, s)` | — | ファイル書き込み (原子的 / UTF-8 BOM 無し) |
| `copy(from, to)` / `remove(p)` | — | コピー / 削除 (remove は欠損を無視) |
| `http_download(url, dest)` | Int (0=成功) | ダウンロード (原子的置換) |
| `upper(s)` / `lower(s)` / `trim(s)` | Text | 文字列変換 |
| `script_path()` / `script_dir()` / `cwd()` | Text | 自スクリプトのパス / ディレクトリ / カレント |

## その他のコンパイル時ルール (`check` で弾かれる)

- **関数はトップレベルでのみ定義**。自身より前に定義した関数のみ呼べる (再帰・前方参照・
  相互再帰は不可)。関数内から外側 (グローバル) 変数と `arg()`/`args()`/`argc()` は使えない。
- **分岐後に使う変数は、全分岐 (else 含む) で同じ型で定義されていること**。
  ```
  if c == 0 { let s = "ok" } else { let s = "ng" }   # 両分岐 Text → 後で使える
  print "{s}"
  ```
  else の無い if で新規変数を定義しても、その変数は if の後では使えない。
- **ループ本体は、ループ前から在る変数の型を変えられない** (ループは複数回実行されるため)。
- **戻り値を捨てられる文は副作用のある呼び出しだけ** (`upper("x")` 単独の行は不可 → `let` で受ける)。
- **比較は連鎖できない** (`a < b < c` は不可 → `a < b and b < c`)。
- `env()` の変数名は英字/`_` 始まりの英数字のみ。`for` のリスト要素に副作用呼び出しは不可。

## 移植性で最重要: 外部コマンドは OS 依存

`run(...)` の仕組みは両 OS 共通だが、**実行するコマンド自体は OS で異なる** (例: `mkdir -p` /
`test` / `rm` は Unix のみ、`cmd` は Windows のみ)。移植性を保つには:

- ファイル操作・ダウンロードは**組み込み関数を使う** (`write_text` / `read_text` / `copy` /
  `remove` / `exists` / `http_download` — 両 OS で同じ動作)。`run(["mkdir", ...])` のような
  OS 依存コマンドは避ける。
- `run(...)` は両 OS に存在するコマンド (例: `git`) に限る。片方だけのコマンドを使うと、その
  スクリプトはその OS 専用になる。

## OS 間の既知の挙動差 (書けるが注意)

- `read_text` / `upper` / `lower` / `trim` は sh 側で末尾改行が除去される (PowerShell は保持)。
- 終了コードは Unix の 0–255 に収める (超過は sh で剰余に丸められる)。
- `write_text` / `copy` / `read_text` の失敗は Windows では致命 (exit 1)、macOS では継続し得る。
  失敗し得る操作は事前に `exists()` で確認する。

## 実例: プロビジョニングスクリプト (両 OS で動く)

組み込み関数 (両 OS 共通) を中心に組み、外部コマンドは両 OS にある `git` に限定している。

```
# 設定ファイルが無ければ生成し、ツールの有無を確認する
let home = env("HOME", ".")     # Windows では HOME が無いことが多い → 既定値 "." が効く
print "home={home}"

let cfg = "applows.config"
if not exists(cfg) {
  write_text(cfg, "generated by applows\nkey=value\n")   # 両 OS で UTF-8/原子的
  print "wrote {cfg}"
} else {
  let body = read_text(cfg)
  print "existing config:"
  print "{body}"
}

let code = run(["git", "--version"])   # git は両 OS にある
if code == 0 {
  print "git present"
} else {
  print "git missing"
  exit 2
}
print "provision done"
exit 0
```

## 書き方の手順 (推奨ループ)

1. 上記構文で `.aplo` を書く。
2. `applows check <file>.aplo` で検証。エラーが出たら本ルールに照らして直す。
3. `applows build <file>.aplo -o <out>.bat` で生成。
4. macOS なら `sh <out>.bat` で動作確認。Windows 実行は対象環境 or CI で。

より詳細な仕様は、このスキルに同梱の完全リファレンス `reference/language.md` を参照
(全構文・全組み込み関数・エッジケースを網羅)。
