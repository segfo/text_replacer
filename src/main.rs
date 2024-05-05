mod dirwalker;
use clap::*;
use clap_num::number_range;
use dirwalker::*;
use once_cell::sync::Lazy;
use std::{
    collections::HashSet,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use toolbox::toml_parser::*;
fn cmd_args_concurrent_parser(s: &str) -> Result<usize, String> {
    number_range(s, 0, 255)
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct CommandLineArgs {
    /// 置換を行うファイルを含むルートディレクトリのパスを指定します。
    root_path: PathBuf,
    /// 最大同時並行数
    #[arg(long, short, default_value_t = 100,value_parser=cmd_args_concurrent_parser)]
    max_concurrent: usize,
}

#[derive(Debug, Serialize, Clone, Deserialize, PartialEq, Eq)]
enum EncodeType {
    NONE,
    XOR,
}

use serde::*;
#[derive(Debug, Serialize, Deserialize, Clone)]
struct Config {
    string: String,
    enc_type: EncodeType,
    enc_key: u8,
    search_ext: HashSet<String>,
    replace_str: String,
}
impl Default for Config {
    fn default() -> Self {
        let mut hs = HashSet::new();
        hs.insert("txt".to_owned());
        hs.insert("res".to_owned());
        hs.insert("req".to_owned());
        let config = Config {
            string: "'J0^/Z?>/$K#/%'JKW/!VH<<VH\x02[:6<>-R,+>1;>-;R>1+6)6-*,R+:,+R963:^[7T7U"
                .to_owned(),
            enc_type: EncodeType::XOR,
            enc_key: 0x7f,
            search_ext: hs,
            replace_str: "<ここにEICAR-TEST-FILE文字列が入ります>".to_owned(),
        };

        config
    }
}
impl Config {
    // ファイルからロードするが、ファイルがなければデフォルトの内容で作成する。
    fn load_file(file: &Path) -> Self {
        let conf = match TomlConfigDeserializer::from_file(file.to_str().unwrap()) {
            Err(_) => {
                let conf = Config::default();
                let _ = TomlConfigSerializer::to_file(conf.clone(), file.to_str().unwrap());
                conf
            }
            Ok(conf) => conf,
        };
        conf
    }
}

static CONFIG: Lazy<Config> = Lazy::new(|| Config::load_file(Path::new("config.toml")));

static EICAR_STR: Lazy<String> = Lazy::new(|| {
    let mut ss = String::new();
    match CONFIG.enc_type {
        EncodeType::NONE => {
            // 何もしない、そのまま渡す
            ss = CONFIG.string.clone();
        }
        EncodeType::XOR => {
            CONFIG
                .string
                .as_bytes()
                .iter()
                .map(|s| char::from(s ^ CONFIG.enc_key))
                .for_each(|c| ss.push(c));
        }
    }
    ss
});

use encoding_rs;

// Callbackに対するパラメータの定義をしている。必要に応じて変更する。
#[derive(Clone, Debug)]
struct CallbackParameter {
    out: StandardLog,
    err_out: ErrorLog,
}

// ログ出力先の実装を行っている。
// WriteTraitをそれぞれに実装し、引数としてCallbackParameterに持たせる。
#[derive(Clone, Debug)]
struct StandardLog {
    out: Arc<Mutex<BufWriter<File>>>,
}
#[derive(Clone, Debug)]
struct ErrorLog {
    err_out: Arc<Mutex<BufWriter<File>>>,
}
impl Write for ErrorLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut lock = self.err_out.lock().unwrap();
        lock.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let mut lock = self.err_out.lock().unwrap();
        lock.flush()
    }
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        let mut lock = self.err_out.lock().unwrap();
        lock.write_all(buf)
    }
    fn write_fmt(&mut self, fmt: std::fmt::Arguments<'_>) -> std::io::Result<()> {
        let mut lock = self.err_out.lock().unwrap();
        lock.write_fmt(fmt)
    }
}

impl Write for StandardLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut lock = self.out.lock().unwrap();
        lock.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let mut lock = self.out.lock().unwrap();
        lock.flush()
    }
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        let mut lock = self.out.lock().unwrap();
        lock.write_all(buf)
    }
    fn write_fmt(&mut self, fmt: std::fmt::Arguments<'_>) -> std::io::Result<()> {
        let mut lock = self.out.lock().unwrap();
        lock.write_fmt(fmt)
    }
}

#[tokio::main]
async fn main() {
    let cmd = CommandLineArgs::parse();

    let out = BufWriter::new(
        OpenOptions::new()
            .truncate(true)
            .write(true)
            .create(true)
            .open("replace.log")
            .unwrap(),
    );

    let err_out = BufWriter::new(
        OpenOptions::new()
            .truncate(true)
            .write(true)
            .create(true)
            .open("replace_error.log")
            .unwrap(),
    );
    let param = CallbackParameter {
        out: StandardLog {
            out: Arc::new(Mutex::new(out)),
        },
        err_out: ErrorLog {
            err_out: Arc::new(Mutex::new(err_out)),
        },
    };
    let mut dw = DirectoryWalker::new(cmd.root_path.to_str().unwrap(), cmd.max_concurrent).unwrap();
    while let Some(next) = dw.pop() {
        let _ = dw.dir_walk_async(&next, callback, param.clone()).await;
    }
}

async fn callback(path: PathBuf, mut param: CallbackParameter) {
    if CONFIG
        .search_ext
        .get(
            path.extension()
                .unwrap_or_default()
                .to_str()
                .unwrap_or_default(),
        )
        .is_some()
    {
        let _ = writeln!(
            param.out,
            "処理を開始しました: {} ...",
            path.to_str().unwrap()
        );
        let path = path.to_owned();

        tokio::spawn(async move {
            if let Err(e) = replacer(&path) {
                let _ = writeln!(
                    param.err_out,
                    "{} は処理が正しく完了しませんでした。ファイルの文字エンコードがSJISまたはUTF-8でない可能性があります。\n    詳細な理由:{}",
                    path.to_str().unwrap(),
                    e.to_string()
                );
            } else {
                let _ = writeln!(
                    param.out,
                    "{} の処理を正常完了しました。",
                    path.to_str().unwrap()
                );
            }
        });
    }
}

fn replacer(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut tmp_path = path.as_os_str().to_owned();
    {
        let mut file = BufReader::new(OpenOptions::new().read(true).write(false).open(path)?);
        let mut tmpfile = None;
        loop {
            tmp_path.push(".tmp");
            if let Ok(writer) = OpenOptions::new()
                .create_new(true)
                .read(false)
                .write(true)
                .open(&tmp_path)
            {
                tmpfile = Some(writer);
                break;
            }
        }
        let mut tmpfile = tmpfile.unwrap();
        loop {
            let mut buf = Vec::new();
            match file.read_until(0x0d, &mut buf) {
                Ok(size) => {
                    if size == 0 {
                        break;
                    }
                    let s = match fallback_charcode(&buf) {
                        Ok(s) => s,
                        Err(_) => match String::from_utf8(buf) {
                            Ok(s) => s,
                            Err(e) => {
                                return Err(Box::new(e));
                            }
                        },
                    };
                    let replaced = s.replace(EICAR_STR.as_str(), &CONFIG.replace_str);
                    let _ = tmpfile.write(replaced.as_bytes());
                }
                Err(e) => {
                    return Err(Box::new(e));
                }
            }
        }
    }
    let _ = std::fs::rename(tmp_path, path);
    Ok(())
}

#[derive(Debug)]
enum StringDecodeErrorKind {
    FromSjis,
}
#[derive(Debug)]
struct StringDecodeError {
    kind: StringDecodeErrorKind,
    msg: String,
}
impl StringDecodeError {
    fn new(kind: StringDecodeErrorKind, msg: &str) -> Self {
        StringDecodeError {
            kind: kind,
            msg: msg.to_owned(),
        }
    }
}
impl std::error::Error for StringDecodeError {}
impl std::fmt::Display for StringDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "")
    }
}

fn fallback_charcode(data: &Vec<u8>) -> Result<String, Box<dyn std::error::Error>> {
    let (decode, _, err) = encoding_rs::SHIFT_JIS.decode(&data);
    if err {
        return Err(Box::new(StringDecodeError::new(
            StringDecodeErrorKind::FromSjis,
            "SJISではありません。",
        )));
    }
    Ok(decode.to_string())
}
