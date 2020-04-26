use failure::Error;
use md5;
use std::fmt;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub enum CacheType {
    Null,
    Local,
}

#[derive(Clone, Debug)]
pub struct LocalCache {
    cache_root: PathBuf,
    precious_hash: md5::Digest,
}

pub trait CacheImplementation {
    fn has_cached_result_for(&self, path: &PathBuf) -> Result<bool, Error>;
}

impl fmt::Debug for dyn CacheImplementation {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "foo",)
    }
}

impl LocalCache {
    fn new(cache_root: &PathBuf, precious_path: &PathBuf) -> Result<LocalCache, Error> {
        Ok(LocalCache {
            cache_root: cache_root.clone(),
            precious_hash: path_hash(precious_path)?,
        })
    }
}

impl CacheImplementation for LocalCache {
    fn has_cached_result_for(&self, path: &PathBuf) -> Result<bool, Error> {
        Ok(true)
    }
}

// This should be safe because we will never modify the same cache entry as
// another thread.
unsafe impl Sync for LocalCache {}

#[derive(Clone, Debug)]
pub struct NullCache {}

impl NullCache {
    fn new() -> NullCache {
        NullCache {}
    }
}

unsafe impl Sync for NullCache {}

impl CacheImplementation for NullCache {
    fn has_cached_result_for(&self, path: &PathBuf) -> Result<bool, Error> {
        Ok(false)
    }
}

fn path_hash(path: &PathBuf) -> Result<md5::Digest, Error> {
    Ok(md5::compute(fs::read(path)?))
}

pub fn new_from_type(
    typ: CacheType,
    root: &PathBuf,
) -> Result<Box<dyn CacheImplementation>, Error> {
    let c: Box<dyn CacheImplementation> = match typ {
        CacheType::Local => Box::new(LocalCache::new(root)?),
        CacheType::Null => Box::new(NullCache::new()),
    };
    Ok(c)
}
