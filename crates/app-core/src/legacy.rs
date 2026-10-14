//! 旧名(Nebula)数据迁移。
//!
//! 1.14.0 起项目更名为 BucketForge,bundle identifier 从 `org.devlive.nebula` 改为
//! `org.devlive.bucketforge`,于是应用数据目录、库文件名、钥匙串服务名都变了。为了让老用户
//! 升级后账号 / 设置 / 书签 / 同步任务 / 已装插件都还在,首次启动时把旧数据**复制**过来:
//!
//! - 数据目录:旧目录与新目录同级(Tauri 的 app_data_dir = 平台数据目录 / identifier),
//!   复制 `nebula.db`(及 SQLite 的 `-wal` / `-shm` / `-journal` 伴生文件)为 `bucketforge.db`,并复制
//!   `plugins/` 插件目录。
//! - 钥匙串:见 [`crate::secret::KeyringSecrets::with_legacy`],新条目不存在时回退读旧条目。
//!
//! 只复制不删除:旧版本仍可回退使用,迁移失败也不影响新版本正常启动。

use std::fs;
use std::io;
use std::path::Path;

/// 旧版 bundle identifier,也是旧应用数据目录名与旧钥匙串服务名。
pub const LEGACY_IDENTIFIER: &str = "org.devlive.nebula";

/// 旧版钥匙串里合并存储密钥的条目名。
pub const LEGACY_SECRETS_KEY: &str = "__nebula_secrets__";

/// 旧版 SQLite 库文件名。
pub const LEGACY_DB_FILE: &str = "nebula.db";

/// 新版 SQLite 库文件名。
pub const DB_FILE: &str = "bucketforge.db";

/// SQLite 主库之外可能存在的伴生文件后缀。
const DB_SIDE_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];

/// 若新数据目录还没有库而旧目录有,把旧库与插件目录复制到新目录。
///
/// 返回是否真的做了迁移。新库已存在(已迁移过或全新安装后用过)时什么都不做。
pub fn migrate_data_dir(legacy_dir: &Path, new_dir: &Path) -> io::Result<bool> {
    let new_db = new_dir.join(DB_FILE);
    let old_db = legacy_dir.join(LEGACY_DB_FILE);
    if new_db.exists() || !old_db.is_file() {
        return Ok(false);
    }
    fs::create_dir_all(new_dir)?;
    // 先复制伴生文件,最后复制主库:中途失败时新目录里没有主库,下次启动会重新迁移。
    for suffix in DB_SIDE_SUFFIXES {
        let side = legacy_dir.join(format!("{LEGACY_DB_FILE}{suffix}"));
        if side.is_file() {
            fs::copy(&side, new_dir.join(format!("{DB_FILE}{suffix}")))?;
        }
    }
    let old_plugins = legacy_dir.join("plugins");
    let new_plugins = new_dir.join("plugins");
    if old_plugins.is_dir() && !new_plugins.exists() {
        copy_dir(&old_plugins, &new_plugins)?;
    }
    fs::copy(&old_db, &new_db)?;
    Ok(true)
}

/// 递归复制目录(不跟随符号链接之外的特殊文件)。
fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("bucketforge-legacy-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn copies_db_side_files_and_plugins() {
        let root = temp_root("copy");
        let old = root.join(LEGACY_IDENTIFIER);
        let new = root.join("org.devlive.bucketforge");
        fs::create_dir_all(old.join("plugins/matting")).unwrap();
        fs::write(old.join(LEGACY_DB_FILE), b"db").unwrap();
        fs::write(old.join("nebula.db-wal"), b"wal").unwrap();
        fs::write(old.join("plugins/matting/model.onnx"), b"model").unwrap();

        assert!(migrate_data_dir(&old, &new).unwrap());
        assert_eq!(fs::read(new.join(DB_FILE)).unwrap(), b"db");
        assert_eq!(fs::read(new.join("bucketforge.db-wal")).unwrap(), b"wal");
        assert!(!new.join("bucketforge.db-shm").exists());
        assert_eq!(
            fs::read(new.join("plugins/matting/model.onnx")).unwrap(),
            b"model"
        );
        // 旧数据保留,便于回退旧版本。
        assert!(old.join(LEGACY_DB_FILE).exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn skips_when_new_db_exists() {
        let root = temp_root("skip");
        let old = root.join(LEGACY_IDENTIFIER);
        let new = root.join("org.devlive.bucketforge");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(old.join(LEGACY_DB_FILE), b"old").unwrap();
        fs::write(new.join(DB_FILE), b"new").unwrap();

        assert!(!migrate_data_dir(&old, &new).unwrap());
        assert_eq!(fs::read(new.join(DB_FILE)).unwrap(), b"new");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn skips_when_no_legacy_data() {
        let root = temp_root("none");
        let new = root.join("org.devlive.bucketforge");
        assert!(!migrate_data_dir(&root.join(LEGACY_IDENTIFIER), &new).unwrap());
        assert!(!new.exists());
    }
}
