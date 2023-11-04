use std::fs;
use std::io;
use std::path::*;
use std::sync::Arc;

pub struct DirectoryInfo {
    path: String,
}

pub struct DirectoryWalker {
    dir_list: Vec<DirectoryInfo>,
    max_concurrent: usize,
}
use tokio::macros::support::Future;
use tokio::task::JoinSet;
//ディレクトリ走査を行うための実装
impl DirectoryWalker {
    pub fn new(root_dir: &str, max_concurrent: usize) -> io::Result<DirectoryWalker> {
        let mut dir_list = Vec::<DirectoryInfo>::new();
        dir_list.push(DirectoryInfo {
            path: root_dir.to_owned(),
        });
        let dw = DirectoryWalker {
            dir_list: dir_list,
            max_concurrent: max_concurrent,
        };
        Ok(dw)
    }

    pub fn dir_walk(&mut self, dir: &DirectoryInfo, callback: fn(entry: &Path)) -> io::Result<()> {
        let dir = Path::new(&dir.path);
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if entry.path().is_dir() {
                self.dir_list.push(DirectoryInfo {
                    path: entry.path().into_os_string().into_string().unwrap(),
                });
            } else {
                callback(&entry.path())
            }
        }
        Ok(())
    }

    pub async fn dir_walk_async<F, Fut,P>(
        &mut self,
        dir: &DirectoryInfo,
        callback: F,
        user_param:P
    ) -> io::Result<()>
    where
        F: Fn(PathBuf,P) -> Fut,
        Fut: Future<Output = ()> + std::marker::Send + 'static,
        P:Clone
    {
        let dir = Path::new(&dir.path);
        let mut join_set = JoinSet::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if entry.path().is_dir() {
                self.dir_list.push(DirectoryInfo {
                    path: entry.path().into_os_string().into_string().unwrap(),
                });
            } else {
                while join_set.len() >= self.max_concurrent {
                    join_set.join_next().await.unwrap().unwrap();
                }
                join_set.spawn(callback(entry.path(),user_param.clone()));
            }
        }
        while join_set.len() > 0 {
            join_set.join_next().await.unwrap().unwrap();
        }
        Ok(())
    }
    pub fn pop(&mut self) -> Option<DirectoryInfo> {
        self.dir_list.pop()
    }
}
