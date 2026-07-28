//! 組み込み関数レジストリ。
//!
//! sema (型検査) と両エミッタ (sh / PowerShell) が同じ定義を共有し、
//! 引数個数・型・戻り型・呼び出し規約の齟齬を防ぐ。

/// 値の型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Text,
    Int,
    Bool,
    /// `List<Text>`。変数へ束縛・補間はできず、`run` の argv と `for` の反復にのみ使える。
    List,
    /// 副作用のみで有用な値を返さない (式として使えない)。
    Unit,
}

impl Type {
    pub fn describe(self) -> &'static str {
        match self {
            Type::Text => "Text",
            Type::Int => "Int",
            Type::Bool => "Bool",
            Type::List => "List",
            Type::Unit => "Unit",
        }
    }
}

/// 組み込み関数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Env,
    Arg,
    Argc,
    Args,
    Run,
    RunCapture,
    Exists,
    IsFile,
    IsDir,
    ReadText,
    ReadStdin,
    WriteText,
    AppendText,
    Copy,
    Remove,
    HttpDownload,
    HttpPost,
    Upper,
    Lower,
    Trim,
    JsonEscape,
    JsonAdd,
    ScriptPath,
    ScriptDir,
    Cwd,
    Hostname,
}

impl Builtin {
    pub fn from_name(name: &str) -> Option<Builtin> {
        Some(match name {
            "env" => Builtin::Env,
            "arg" => Builtin::Arg,
            "argc" => Builtin::Argc,
            "args" => Builtin::Args,
            "run" => Builtin::Run,
            "run_capture" => Builtin::RunCapture,
            "exists" => Builtin::Exists,
            "is_file" => Builtin::IsFile,
            "is_dir" => Builtin::IsDir,
            "read_text" => Builtin::ReadText,
            "read_stdin" => Builtin::ReadStdin,
            "write_text" => Builtin::WriteText,
            "append_text" => Builtin::AppendText,
            "copy" => Builtin::Copy,
            "remove" => Builtin::Remove,
            "http_download" => Builtin::HttpDownload,
            "http_post" => Builtin::HttpPost,
            "upper" => Builtin::Upper,
            "lower" => Builtin::Lower,
            "trim" => Builtin::Trim,
            "json_escape" => Builtin::JsonEscape,
            "json_add" => Builtin::JsonAdd,
            "script_path" => Builtin::ScriptPath,
            "script_dir" => Builtin::ScriptDir,
            "cwd" => Builtin::Cwd,
            "hostname" => Builtin::Hostname,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Builtin::Env => "env",
            Builtin::Arg => "arg",
            Builtin::Argc => "argc",
            Builtin::Args => "args",
            Builtin::Run => "run",
            Builtin::RunCapture => "run_capture",
            Builtin::Exists => "exists",
            Builtin::IsFile => "is_file",
            Builtin::IsDir => "is_dir",
            Builtin::ReadText => "read_text",
            Builtin::ReadStdin => "read_stdin",
            Builtin::WriteText => "write_text",
            Builtin::AppendText => "append_text",
            Builtin::Copy => "copy",
            Builtin::Remove => "remove",
            Builtin::HttpDownload => "http_download",
            Builtin::HttpPost => "http_post",
            Builtin::Upper => "upper",
            Builtin::Lower => "lower",
            Builtin::Trim => "trim",
            Builtin::JsonEscape => "json_escape",
            Builtin::JsonAdd => "json_add",
            Builtin::ScriptPath => "script_path",
            Builtin::ScriptDir => "script_dir",
            Builtin::Cwd => "cwd",
            Builtin::Hostname => "hostname",
        }
    }

    /// 期待する引数の型。
    pub fn params(self) -> &'static [Type] {
        match self {
            Builtin::Env => &[Type::Text, Type::Text],
            Builtin::Arg => &[Type::Int],
            Builtin::Argc => &[],
            Builtin::Args => &[],
            Builtin::Run => &[Type::List],
            Builtin::RunCapture => &[Type::List, Type::Text],
            Builtin::Exists | Builtin::IsFile | Builtin::IsDir => &[Type::Text],
            Builtin::ReadText => &[Type::Text],
            Builtin::ReadStdin => &[],
            Builtin::WriteText | Builtin::AppendText => &[Type::Text, Type::Text],
            Builtin::Copy => &[Type::Text, Type::Text],
            Builtin::Remove => &[Type::Text],
            Builtin::HttpDownload => &[Type::Text, Type::Text],
            Builtin::HttpPost => &[Type::Text, Type::List, Type::Text],
            Builtin::Upper | Builtin::Lower | Builtin::Trim | Builtin::JsonEscape => &[Type::Text],
            Builtin::JsonAdd => &[Type::Text, Type::Text, Type::Text],
            Builtin::ScriptPath | Builtin::ScriptDir | Builtin::Cwd | Builtin::Hostname => &[],
        }
    }

    /// 戻り型。
    pub fn ret(self) -> Type {
        match self {
            Builtin::Env => Type::Text,
            Builtin::Arg => Type::Text,
            Builtin::Argc => Type::Int,
            Builtin::Args => Type::List,
            Builtin::Run => Type::Int,
            Builtin::RunCapture => Type::Text,
            Builtin::Exists | Builtin::IsFile | Builtin::IsDir => Type::Bool,
            Builtin::ReadText | Builtin::ReadStdin => Type::Text,
            Builtin::WriteText | Builtin::AppendText | Builtin::Copy | Builtin::Remove => {
                Type::Unit
            }
            Builtin::HttpDownload | Builtin::HttpPost => Type::Int,
            Builtin::Upper
            | Builtin::Lower
            | Builtin::Trim
            | Builtin::JsonEscape
            | Builtin::JsonAdd => Type::Text,
            Builtin::ScriptPath | Builtin::ScriptDir | Builtin::Cwd | Builtin::Hostname => {
                Type::Text
            }
        }
    }

    /// 第 1 引数が文字列リテラル必須か (env の変数名など)。
    pub fn requires_literal_first_arg(self) -> bool {
        matches!(self, Builtin::Env)
    }

    /// 引数がリテラル整数必須か (arg のインデックスなど)。
    pub fn requires_literal_int_arg(self) -> bool {
        matches!(self, Builtin::Arg)
    }

    /// 副作用を持つか。条件式 (`and` / `or` / `not` の内側) へ書けないかの判定に使う。
    /// `read_stdin` は標準入力を消費し 2 回目が空になるため副作用扱いにする。
    pub fn is_side_effecting(self) -> bool {
        matches!(
            self,
            Builtin::WriteText
                | Builtin::AppendText
                | Builtin::Copy
                | Builtin::Remove
                | Builtin::Run
                | Builtin::RunCapture
                | Builtin::ReadStdin
                | Builtin::HttpDownload
                | Builtin::HttpPost
        )
    }

    /// 戻り値を捨てる単独の文として書けるか。
    /// 副作用があっても戻り値こそが目的の組み込み (`run_capture`) は捨てさせない
    /// — 出力が要らないなら `run` を使うべきで、捨てると stdout も stderr も
    /// 消える紛らわしい処理になるため。
    pub fn discardable_as_statement(self) -> bool {
        self.is_side_effecting() && !matches!(self, Builtin::RunCapture)
    }
}
