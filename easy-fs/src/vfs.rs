use super::{
    block_cache_sync_all, get_block_cache, BlockDevice, DirEntry, DiskInode, DiskInodeType,
    EasyFileSystem, DIRENT_SZ,
};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::{Mutex, MutexGuard};
/// Virtual filesystem layer over easy-fs
pub struct Inode {
    inode_id: u32,
    block_id: usize,
    block_offset: usize,
    fs: Arc<Mutex<EasyFileSystem>>,
    block_device: Arc<dyn BlockDevice>,
}

impl Inode {
    /// Create a vfs inode
    pub fn new(
        inode_id: u32,
        block_id: u32,
        block_offset: usize,
        fs: Arc<Mutex<EasyFileSystem>>,
        block_device: Arc<dyn BlockDevice>,
    ) -> Self {
        Self {
            inode_id,
            block_id: block_id as usize,
            block_offset,
            fs,
            block_device,
        }
    }
    /// Call a function over a disk inode to read it
    fn read_disk_inode<V>(&self, f: impl FnOnce(&DiskInode) -> V) -> V {
        get_block_cache(self.block_id, Arc::clone(&self.block_device))
            .lock()
            .read(self.block_offset, f)
    }
    /// Call a function over a disk inode to modify it
    fn modify_disk_inode<V>(&self, f: impl FnOnce(&mut DiskInode) -> V) -> V {
        get_block_cache(self.block_id, Arc::clone(&self.block_device))
            .lock()
            .modify(self.block_offset, f)
    }
    /// Find inode under a disk inode by name
    fn find_inode_id(&self, name: &str, disk_inode: &DiskInode) -> Option<u32> {
        self.find_inode_id_and_entry_id(name, disk_inode)
            .map(|(_, inode_id)| inode_id)
    }
    fn find_inode_id_and_entry_id(
        &self,
        name: &str,
        disk_inode: &DiskInode,
    ) -> Option<(usize, u32)> {
        // assert it is a directory
        assert!(disk_inode.is_dir());
        let file_count = (disk_inode.size as usize) / DIRENT_SZ;
        let mut dirent = DirEntry::empty();
        for i in 0..file_count {
            assert_eq!(
                disk_inode.read_at(DIRENT_SZ * i, dirent.as_bytes_mut(), &self.block_device,),
                DIRENT_SZ,
            );
            if !dirent.name().is_empty() && dirent.name() == name {
                return Some((i, dirent.inode_id() as u32));
            }
        }
        None
    }
    fn find_empty_entry_id(&self, disk_inode: &DiskInode) -> Option<usize> {
        assert!(disk_inode.is_dir());
        let file_count = (disk_inode.size as usize) / DIRENT_SZ;
        let mut dirent = DirEntry::empty();
        for i in 0..file_count {
            assert_eq!(
                disk_inode.read_at(DIRENT_SZ * i, dirent.as_bytes_mut(), &self.block_device,),
                DIRENT_SZ,
            );
            if dirent.name().is_empty() {
                return Some(i);
            }
        }
        None
    }
    fn insert_dirent(
        &self,
        root_inode: &mut DiskInode,
        entry_id: usize,
        dirent: DirEntry,
        fs: &mut MutexGuard<EasyFileSystem>,
    ) {
        let file_count = (root_inode.size as usize) / DIRENT_SZ;
        if entry_id == file_count {
            self.increase_size(((file_count + 1) * DIRENT_SZ) as u32, root_inode, fs);
        }
        assert_eq!(
            root_inode.write_at(entry_id * DIRENT_SZ, dirent.as_bytes(), &self.block_device),
            DIRENT_SZ,
        );
    }
    fn clear_locked(&self, disk_inode: &mut DiskInode, fs: &mut MutexGuard<EasyFileSystem>) {
        let size = disk_inode.size;
        let data_blocks_dealloc = disk_inode.clear_size(&self.block_device);
        assert_eq!(
            data_blocks_dealloc.len(),
            DiskInode::total_blocks(size) as usize
        );
        for data_block in data_blocks_dealloc.into_iter() {
            fs.dealloc_data(data_block);
        }
    }
    /// Find inode under current inode by name
    pub fn find(&self, name: &str) -> Option<Arc<Inode>> {
        let fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            self.find_inode_id(name, disk_inode).map(|inode_id| {
                let (block_id, block_offset) = fs.get_disk_inode_pos(inode_id);
                Arc::new(Self::new(
                    inode_id,
                    block_id,
                    block_offset,
                    self.fs.clone(),
                    self.block_device.clone(),
                ))
            })
        })
    }
    /// Increase the size of a disk inode
    fn increase_size(
        &self,
        new_size: u32,
        disk_inode: &mut DiskInode,
        fs: &mut MutexGuard<EasyFileSystem>,
    ) {
        if new_size < disk_inode.size {
            return;
        }
        let blocks_needed = disk_inode.blocks_num_needed(new_size);
        let mut v: Vec<u32> = Vec::new();
        for _ in 0..blocks_needed {
            v.push(fs.alloc_data());
        }
        disk_inode.increase_size(new_size, v, &self.block_device);
    }
    /// Create inode under current inode by name
    pub fn create(&self, name: &str) -> Option<Arc<Inode>> {
        let mut fs = self.fs.lock();
        let op = |root_inode: &DiskInode| -> Option<usize> {
            // assert it is a directory
            assert!(root_inode.is_dir());
            // has the file been created?
            if self.find_inode_id(name, root_inode).is_some() {
                None
            } else {
                Some(
                    self.find_empty_entry_id(root_inode)
                        .unwrap_or((root_inode.size as usize) / DIRENT_SZ),
                )
            }
        };
        let entry_id = self.read_disk_inode(op)?;
        // create a new file
        // alloc a inode with an indirect block
        let new_inode_id = fs.alloc_inode();
        // initialize inode
        let (new_inode_block_id, new_inode_block_offset) = fs.get_disk_inode_pos(new_inode_id);
        get_block_cache(new_inode_block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .modify(new_inode_block_offset, |new_inode: &mut DiskInode| {
                new_inode.initialize(DiskInodeType::File);
            });
        self.modify_disk_inode(|root_inode| {
            self.insert_dirent(
                root_inode,
                entry_id,
                DirEntry::new(name, new_inode_id),
                &mut fs,
            );
        });

        let (block_id, block_offset) = fs.get_disk_inode_pos(new_inode_id);
        block_cache_sync_all();
        // return inode
        Some(Arc::new(Self::new(
            new_inode_id,
            block_id,
            block_offset,
            self.fs.clone(),
            self.block_device.clone(),
        )))
        // release efs lock automatically by compiler
    }
    /// Create a hard link for an inode under the current directory.
    pub fn link(&self, old_name: &str, new_name: &str) -> bool {
        if old_name.is_empty() || new_name.is_empty() || old_name == new_name {
            return false;
        }
        let mut fs = self.fs.lock();
        let Some((old_inode_id, entry_id)) = self.read_disk_inode(|root_inode| {
            assert!(root_inode.is_dir());
            if self.find_inode_id(new_name, root_inode).is_some() {
                return None;
            }
            let old_inode_id = self.find_inode_id(old_name, root_inode)?;
            let entry_id = self
                .find_empty_entry_id(root_inode)
                .unwrap_or((root_inode.size as usize) / DIRENT_SZ);
            Some((old_inode_id, entry_id))
        }) else {
            return false;
        };
        let (block_id, block_offset) = fs.get_disk_inode_pos(old_inode_id);
        let linked = get_block_cache(block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .modify(block_offset, |disk_inode: &mut DiskInode| {
                if disk_inode.is_dir() {
                    false
                } else {
                    disk_inode.inc_nlink();
                    true
                }
            });
        if !linked {
            return false;
        }
        self.modify_disk_inode(|root_inode| {
            self.insert_dirent(
                root_inode,
                entry_id,
                DirEntry::new(new_name, old_inode_id),
                &mut fs,
            );
        });
        block_cache_sync_all();
        true
    }
    /// Remove a hard link under the current directory.
    pub fn unlink(&self, name: &str) -> bool {
        if name.is_empty() {
            return false;
        }
        let mut fs = self.fs.lock();
        let Some((entry_id, inode_id)) = self.read_disk_inode(|root_inode| {
            assert!(root_inode.is_dir());
            self.find_inode_id_and_entry_id(name, root_inode)
        }) else {
            return false;
        };
        let (block_id, block_offset) = fs.get_disk_inode_pos(inode_id);
        let should_dealloc = get_block_cache(block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .modify(block_offset, |disk_inode: &mut DiskInode| {
                if disk_inode.is_dir() || disk_inode.nlink() == 0 {
                    None
                } else {
                    disk_inode.dec_nlink();
                    if disk_inode.nlink() == 0 {
                        self.clear_locked(disk_inode, &mut fs);
                    }
                    Some(disk_inode.nlink() == 0)
                }
            });
        let Some(should_dealloc) = should_dealloc else {
            return false;
        };
        self.modify_disk_inode(|root_inode| {
            assert_eq!(
                root_inode.write_at(
                    entry_id * DIRENT_SZ,
                    DirEntry::empty().as_bytes(),
                    &self.block_device,
                ),
                DIRENT_SZ,
            );
        });
        if should_dealloc {
            fs.dealloc_inode(inode_id);
        }
        block_cache_sync_all();
        true
    }
    /// Get the inode metadata needed by the OS.
    pub fn stat(&self) -> (u32, bool, u32) {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| (self.inode_id, disk_inode.is_dir(), disk_inode.nlink()))
    }
    /// List inodes under current inode
    pub fn ls(&self) -> Vec<String> {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            let file_count = (disk_inode.size as usize) / DIRENT_SZ;
            let mut v: Vec<String> = Vec::new();
            for i in 0..file_count {
                let mut dirent = DirEntry::empty();
                assert_eq!(
                    disk_inode.read_at(i * DIRENT_SZ, dirent.as_bytes_mut(), &self.block_device,),
                    DIRENT_SZ,
                );
                if !dirent.name().is_empty() {
                    v.push(String::from(dirent.name()));
                }
            }
            v
        })
    }
    /// Read data from current inode
    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| disk_inode.read_at(offset, buf, &self.block_device))
    }
    /// Write data to current inode
    pub fn write_at(&self, offset: usize, buf: &[u8]) -> usize {
        let mut fs = self.fs.lock();
        let size = self.modify_disk_inode(|disk_inode| {
            self.increase_size((offset + buf.len()) as u32, disk_inode, &mut fs);
            disk_inode.write_at(offset, buf, &self.block_device)
        });
        block_cache_sync_all();
        size
    }
    /// Clear the data in current inode
    pub fn clear(&self) {
        let mut fs = self.fs.lock();
        self.modify_disk_inode(|disk_inode| self.clear_locked(disk_inode, &mut fs));
        block_cache_sync_all();
    }
}
