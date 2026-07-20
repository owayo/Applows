---
name: applows
description: >-
  Applows 言語 (.aplo) を書くためのリファレンス。シェル風の 1 ソースから、バニラ
  Windows 11 と macOS の両方で追加ランタイム不要で動く単一ポリグロット .bat を生成する
  コンパイラ。Windows と macOS の両対応スクリプト・クロスプラットフォームの
  ブートストラップ / プロビジョニング / セットアップスクリプトを書く、`.aplo` を書く/直す、
  「applows」でコンパイルする場面で発動。型付き・安全側の言語なので、書いたら必ず
  `applows check` で検証する。
allowed-tools: Bash(applows:*)
---

# Applows

Applows はシェル風の 1 ソース (`.aplo`) から、**バニラ Windows 11 と macOS の両方で
追加ランタイムなしにネイティブ実行できる単一のポリグロットスクリプト (`.bat`)** を生成する
コンパイラ。出力 1 ファイルが Windows Batch + PowerShell 5.1 + macOS `/bin/sh`(bash)+zsh
として同時に valid。用途はクロス OS のブートストラップ / プロビジョニング / CI セットアップ。

**Applows は型付きで安全側に倒した言語**。暗黙の型変換や truthiness が無く、危険パターンは
コンパイルエラーで弾かれる。**書いたら必ず `applows check <file>.aplo` で検証**し、エラーが
出たら下記ルールに従って直す。これが正しいコードを書く最短ルート。

## CLI

```bash
applows build  input.aplo [-o out.bat]        # 単一ポリグロット .bat を生成 (出力既定は .bat)
applows check  input.aplo                      # 型検査のみ (出力しない) — 書いたら必ずこれで検証
applows emit   input.aplo --target sh|powershell|ir   # 中間生成物を確認 (デバッグ用)
```
生成した `.bat` は macOS では `sh out.bat` / `./out.bat`、Windows では `out.bat` で実行。

## 最初に覚える鉄則 (コンパイルエラーを避ける)

AI が最も踏みやすい落とし穴。ここを外すと `check` が通らない。

1. **文字列補間 `{...}` は変数名のみ**。関数結果や式は入れられない。
   ```
   let d = script_dir()
   print "dir={d}"          # OK
   # print "dir={script_dir()}"   # ← 不可
   ```
2. **外部コマンドは argv 配列**: `run(["git", "status"])`。文字列 1 本 `run("git status")` は不可。
3. **条件は真偽値だけ** (truthiness 無し)。コマンドの成否は `== 0` で明示。
   ```
   let code = run(["git", "--version"])
   if code == 0 { print "ok" }        # OK
   # if run(["git"]) { ... }          # ← 不可 (Int は条件にできない)
   ```
4. **`and` / `or` / `not` の内側に副作用呼び出し** (`run` / `http_download` / ユーザ関数) は書けない。
   先に `let` で受ける。
   ```
   let c = run(["test", "-d", "/tmp"])
   if c == 0 and enabled == 1 { ... }   # OK
   # if run([...]) == 0 and x { ... }    # ← 不可
   ```
5. **文字列の比較は `==` `!=` のみ** (大小比較 `< >` は Int 専用)。
6. **リストは変数へ代入・補間できない**。`run(...)` の argv と `for` の反復にだけ使う。
7. **算術・比較は同じ型同士**。`Text` と `Int` は混ぜられない (自動変換なし)。数値引数を計算に
   使いたい場合でも文字列→整数変換は無い (MVP)。

## 型

| 型 | 説明 |
|---|---|
| `Text` | 文字列 (UTF-8)。`"..."` |
| `Int` | 64bit 整数。`42`, `-3` |
| `Bool` | 条件の中だけに存在。変数へ代入・格納できない。 |
| `List` | 文字列リスト。`["a", "b"]`。argv と for 反復専用。 |

## 構文

```
# コメント (# から行末)

let name = "world"            # 変数の宣言/再代入
let count = 3

print "hello, {name}!"        # 出力 (改行付き)。補間は {変数名} のみ
print "count={count} x2={count}"

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

# while
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

# 関数: 値渡し / パラメータは Text 扱い / 戻り値は終了ステータス(Int) / 再帰・前方参照・外側変数参照は不可
fn greet(who) {
  print "hi {who}"
  return 0
}
greet("team")

exit 0                        # 終了コード (0-255)
```

## 組み込み関数

| 関数 | 戻り | 用途 |
|---|---|---|
| `print EXPR` (文) | — | 改行付き出力 |
| `run(list)` | Int (終了コード) | 外部コマンド実行 (argv 配列)。stdio は継承 |
| `env(name, default)` | Text | 環境変数。`name` は識別子リテラル (`"PATH"` 等) |
| `arg(i)` / `argc()` / `args()` | Text / Int / List | スクリプト引数 (1 始まり)。**トップレベルのみ**、関数内不可 |
| `exists(p)` / `is_file(p)` / `is_dir(p)` | Bool | 存在判定 (条件でのみ使用) |
| `read_text(p)` | Text | ファイル読み込み (UTF-8) |
| `write_text(p, s)` / `append_text(p, s)` | — | ファイル書き込み (原子的 / UTF-8 BOM 無し) |
| `copy(from, to)` / `remove(p)` | — | コピー / 削除 (remove は欠損を無視) |
| `http_download(url, dest)` | Int (0=成功) | ダウンロード (原子的置換) |
| `upper(s)` / `lower(s)` / `trim(s)` | Text | 文字列変換 |
| `script_path()` / `script_dir()` / `cwd()` | Text | 自スクリプトのパス / ディレクトリ / カレント |

## その他のコンパイル時ルール (`check` で弾かれる)

- **関数**: 自身より前に定義した関数のみ呼べる (再帰・前方参照・相互再帰は不可)。関数内から
  外側 (グローバル) 変数は見えない。引数として渡す。戻り値は `return <Int>` (ステータス)。
- **`if`/`while`/`for` の分岐で型が食い違う変数**を分岐後に使うと不可。全分岐で同じ型なら可。
  ```
  if c == 0 { let s = "ok" } else { let s = "ng" }   # 両分岐 Text
  print "{s}"                                          # OK
  ```
- **ループ本体は、ループ前から在る変数の型を変えられない** (毎周一定である必要)。
- **`arg()` のインデックスは 1 以上**。`env()` の変数名は英字/`_` 始まりの英数字のみ。
- **`for` のリスト要素に副作用呼び出しは書けない** (先に `let` で受ける)。

## 移植性で最重要: 外部コマンドは OS 依存

`run(...)` の仕組みは両 OS 共通だが、**実行するコマンド自体は OS で異なる** (例: `mkdir -p` は
Unix のみ、`cmd` は Windows のみ)。Applows に OS 分岐の構文は無い (MVP)。移植性を保つには:

- ファイル操作・ダウンロードは**組み込み関数を使う** (`write_text` / `read_text` / `copy` /
  `remove` / `exists` / `http_download` — これらは両 OS で同じ動作)。`run(["mkdir", ...])` の
  ような OS 依存コマンドは避ける。
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
let home = env("HOME", ".")
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
