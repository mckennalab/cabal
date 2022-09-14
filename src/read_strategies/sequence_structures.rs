use bio::io::fastq::{Record, Records};
use bio::io::fastq;
use core::clone::Clone;
use core::option::Option;
use core::option::Option::{None, Some};
use std::fs::File;
use std::path::{Path, PathBuf};
use crate::sorters::sorter::ReadSortingOnDiskContainer;
use std::io::BufReader;

pub struct ReadSetContainer {
    pub read_one: Record,
    pub read_two: Option<Record>,
    pub index_one: Option<Record>,
    pub index_two: Option<Record>,
}

pub struct SequenceSetContainer {
    pub read_one: Vec<u8>,
    pub read_two: Option<Vec<u8>>,
    pub index_one: Option<Vec<u8>>,
    pub index_two: Option<Vec<u8>>,
}

impl Clone for ReadSetContainer {
    fn clone(&self) -> ReadSetContainer {
        ReadSetContainer {
            read_one: self.read_one.clone(),
            read_two: if self.read_two.as_ref().is_some() {Some(self.read_two.as_ref().unwrap().clone())} else {None},
            index_one: if self.index_one.as_ref().is_some() {Some(self.index_one.as_ref().unwrap().clone())} else {None},
            index_two: if self.index_two.as_ref().is_some() {Some(self.index_two.as_ref().unwrap().clone())} else {None}
        }
    }
}

impl ReadSetContainer {
    pub fn new_from_read1(rec: Record) -> ReadSetContainer {
        ReadSetContainer{
            read_one: rec,
            read_two: None,
            index_one: None,
            index_two: None
        }
    }
}

impl std::fmt::Display for ReadSetContainer {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let res = write!(f,"{}",&self.read_one);
        if let Some(x) = &self.read_two {
            write!(f,"{}",x);
        }
        if let Some(x) = &self.index_one {
            write!(f,"{}",x);
        }
        if let Some(x) = &self.index_two {
            write!(f,"{}",x);
        }
        res
    }
}

pub struct ReadFileContainer {
    pub read_one: PathBuf,
    pub read_two: PathBuf,
    pub index_one: PathBuf,
    pub index_two: PathBuf,
}

pub struct ReadIterator {
    pub read_one: Records<BufReader<File>>,
    pub read_two: Option<Records<BufReader<File>>>,
    pub index_one: Option<Records<BufReader<File>>>,
    pub index_two: Option<Records<BufReader<File>>>,
}

impl Iterator for ReadIterator {
    type Item = ReadSetContainer;

    fn next(&mut self) -> Option<ReadSetContainer> {
        let next_read_one = self.read_one.next();
        if next_read_one.is_some() {
            Some(ReadSetContainer {
                read_one: next_read_one.unwrap().unwrap(),
                read_two: match self.read_two
                {
                    Some(ref mut read_pointer) => Some(read_pointer.next().unwrap().unwrap()),
                    None => None,
                },
                index_one: match self.index_one
                {
                    Some(ref mut read_pointer) => Some(read_pointer.next().unwrap().unwrap()),
                    None => None,
                },
                index_two: match self.index_two
                {
                    Some(ref mut read_pointer) => Some(read_pointer.next().unwrap().unwrap()),
                    None => None,
                },
            })
        } else {
            None
        }
    }
}

impl ReadIterator
{
    pub fn new(read_1: &Path,
               read_2: &Path,
               index_1: &Path,
               index_2: &Path,
    ) -> ReadIterator  {
        ReadIterator {
            read_one: ReadIterator::open_reader(&Some(read_1)).unwrap(),
            read_two: ReadIterator::open_reader(&Some(read_2)),
            index_one: ReadIterator::open_reader(&Some(index_1)),
            index_two: ReadIterator::open_reader(&Some(index_2)),
        }
    }

    pub fn new_from_bundle(read_files: &ReadFileContainer) -> ReadIterator  {
        ReadIterator::new(&read_files.read_one, &read_files.read_two, &read_files.index_one, &read_files.index_two)
    }

    pub fn new_from_on_disk_sorter(read_sorter: &ReadSortingOnDiskContainer) -> ReadIterator {
        let path1 = read_sorter.file_1.as_path();
        let path2 = if read_sorter.file_2.is_some() {Some(read_sorter.file_2.as_ref().unwrap().as_path())} else {None};
        let path3 = if read_sorter.file_3.is_some() {Some(read_sorter.file_3.as_ref().unwrap().as_path())} else {None};
        let path4 = if read_sorter.file_4.is_some() {Some(read_sorter.file_4.as_ref().unwrap().as_path())} else {None};
        ReadIterator {
            read_one: ReadIterator::open_reader(&Some(&path1)).unwrap(),
            read_two: ReadIterator::open_reader(&path2),
            index_one: ReadIterator::open_reader(&path3),
            index_two: ReadIterator::open_reader(&path4)
        }
    }

    fn open_reader(check_path: &Option<&Path>) -> Option<Records<BufReader<File>>> {
        if check_path.is_some() && check_path.as_ref().unwrap().exists() {
            let mut f2gz = fastq::Reader::new(File::open(check_path.unwrap()).unwrap());
            let records = f2gz.records();
            Some(records)
        } else {
            None
        }
    }
}
