use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use crate::read_strategies::sequence_file_containers::{ReadPattern, ReadSetContainer};
use crate::read_strategies::sequence_file_containers::OutputReadSetWriter;
use crate::RunSpecifications;
use indicatif::style::ProgressTracker;
use std::borrow::{BorrowMut, Borrow};
use crate::read_strategies::sequence_file_containers::ReadFileContainer;
use crate::read_strategies::sequence_file_containers::SuperClusterOnDiskIterator;
use crate::read_strategies::sequence_file_containers::ReadIterator;
use crate::read_strategies::sequence_file_containers::ClusteredReads;
use flate2::write::GzEncoder;
use std::fs::File;
use std::io::Write;


pub struct RoundRobinDiskWriter {
    writers: Vec<GzEncoder<File>>,
    underlying_files: Vec<PathBuf>,
    assigned_sequences: HashMap<Vec<u8>, usize>,
    assigned_sequences_count: HashMap<Vec<u8>, usize>,
    output_counts: HashMap<usize, usize>,
    current_bin: usize,
    read_pattern: ReadPattern,
    run_specs: RunSpecifications,
}

impl RoundRobinDiskWriter {
    pub fn from(run_specs: &mut RunSpecifications, read_pattern: &ReadPattern) -> RoundRobinDiskWriter {
        let mut writers = Vec::new();
        let mut underlying_files = Vec::new();
        let mut assigned_sequences: HashMap<Vec<u8>, usize> = HashMap::new();
        let mut assigned_sequences_count: HashMap<Vec<u8>, usize> = HashMap::new();

        let mut output_counts: HashMap<usize, usize> = HashMap::new();

        for i in 0..run_specs.sorting_file_count {
            let temp_file = run_specs.create_temp_file();
            println!("setting up input file: {:?}",temp_file);
            underlying_files.push(temp_file.clone());
            let mut writer: GzEncoder<File> = OutputReadSetWriter::create_writer(&temp_file);
            ClusteredReads::write_header(&mut writer, read_pattern, -1);
            writers.push(writer);
            output_counts.insert(i, 0);
        }

        RoundRobinDiskWriter {
            writers,
            underlying_files,
            assigned_sequences,
            assigned_sequences_count,
            output_counts,
            current_bin: 0,
            read_pattern: read_pattern.clone(),
            run_specs: run_specs.clone(),
        }
    }

    pub fn write_all_reads(writer: &mut GzEncoder<File>, read: &ReadSetContainer) {
        write!(writer,"{}",read.read_one.to_string());
        if let Some(x) = &read.read_two {write!(writer,"{}",x.to_string());}
        if let Some(x) = &read.index_one {write!(writer,"{}",x.to_string());}
        if let Some(x) = &read.index_two {write!(writer,"{}",x.to_string());}
    }

    pub fn write(&mut self, sort_string: &Vec<u8>, read: &ReadSetContainer) {
        match self.assigned_sequences.get(sort_string) {
            Some(x) => {
                let mut writer = self.writers.get_mut(*x).unwrap();
                //println!("1opening the file {:?}", &self.underlying_files.get(*x));
                self.assigned_sequences_count.insert(
                    sort_string.clone(),
                    self.assigned_sequences_count.get(sort_string).unwrap_or(&(0 as usize)) + 1
                );
                RoundRobinDiskWriter::write_all_reads(&mut writer, read);
                self.output_counts.insert(*x,
                                          self.output_counts.get(x).unwrap_or(&(0 as usize)) + 1);
            },
            _ => {
                self.assigned_sequences.insert(sort_string.clone(),self.current_bin);
                self.output_counts.insert(self.current_bin,
                                          self.output_counts.get(&self.current_bin).unwrap_or(&(0 as usize)) + 1);
                //println!("2opening the file {:?}", &self.underlying_files.get(self.current_bin));
                RoundRobinDiskWriter::write_all_reads(&mut self.writers.get_mut(self.current_bin).unwrap(), read);
                self.current_bin += 1;
                if self.current_bin >= self.output_counts.len() {
                    self.current_bin = 0;
                }
            }
        }
    }

    pub fn get_writers(self) -> SuperClusterOnDiskIterator {
        for writer in self.writers {
            drop(writer);
        };

        for i in 0..self.underlying_files.len() {
            println!("bin = {} size = {}, file {:?}", i, self.output_counts.get(&i).unwrap(), self.underlying_files.get(i).unwrap());
        }
        println!("lookup set size = {}",self.assigned_sequences.len());
        for (s,i) in self.assigned_sequences.iter() {
            println!("seq = {}, {} count {}", String::from_utf8(s.clone()).unwrap(), i, self.assigned_sequences_count.get(s).unwrap_or(&0));

        }
        SuperClusterOnDiskIterator::new_from_read_file_container(
            self.underlying_files,
            self.read_pattern.clone(),
            self.run_specs.clone())
    }
}