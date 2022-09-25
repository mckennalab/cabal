use std::borrow::BorrowMut;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufWriter;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::slice::Iter;
use std::sync::{Arc, Mutex};

use flate2::Compression;
use flate2::write::GzEncoder;
use log::{info, warn};
use rust_htslib::bgzf::Writer;
use rayon::prelude::*;

use crate::consensus::consensus_builders::create_seq_layout_poa_consensus;
use crate::read_strategies::sequence_file_containers::{ReadFileContainer, ReadIterator, ReadSetContainer};
use crate::read_strategies::sequence_file_containers::ClusteredReadIterator;
use crate::read_strategies::sequence_file_containers::ClusteredReads;
use crate::read_strategies::sequence_file_containers::OutputReadSetWriter;
use crate::read_strategies::sequence_layout::LayoutType;
use crate::read_strategies::sequence_layout::SequenceLayout;
use crate::read_strategies::sequence_layout::transform;
use crate::RunSpecifications;
use crate::sorters::known_list::KnownList;
use crate::sorters::known_list::KnownListBinSplit;
use crate::sorters::known_list::KnownListConsensus;
use crate::sorters::known_list::KnownListDiskStream;
use crate::sorters::sort_streams::*;

#[derive(Clone)]
pub enum SortStructure {
    KNOWN_LIST { layout_type: LayoutType, max_distance: usize, on_disk: bool, known_list: Arc<Mutex<KnownList>> },
    HD_UMI { layout_type: LayoutType, max_distance: usize, on_disk: bool },
    LD_UMI { layout_type: LayoutType, max_distance: usize, on_disk: bool },
}

impl std::fmt::Display for SortStructure {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            SortStructure::KNOWN_LIST { layout_type, max_distance: maximum_distance, on_disk, known_list } => {
                let res = write!(f, "KNOWN_LIST,{},{},{}", layout_type, maximum_distance, on_disk);
                res
            }
            SortStructure::HD_UMI { layout_type, max_distance, on_disk } => {
                let res = write!(f, "HD_UMI,{},{},{}", layout_type, max_distance, on_disk);
                res
            }
            SortStructure::LD_UMI { layout_type, max_distance, on_disk } => {
                let res = write!(f, "LD_UMI,{},{},{}", layout_type, max_distance, on_disk);
                res
            }
        }
    }
}


impl SortStructure {
    pub fn from_layout(layout: &LayoutType, known_lists: HashMap<LayoutType, Arc<Mutex<KnownList>>>) -> Vec<SortStructure> {
        match layout {
            LayoutType::TENXV3 => {
                let mut ret = Vec::new();
                ret.push(SortStructure::KNOWN_LIST { layout_type: LayoutType::TENXV3, max_distance: 1, on_disk: true, known_list: known_lists.get(&LayoutType::TENXV3).unwrap().clone() });
                ret.push(SortStructure::LD_UMI { layout_type: LayoutType::TENXV3, max_distance: 3, on_disk: false });
                ret
            }
            LayoutType::TENXV2 => {
                let mut ret = Vec::new();
                ret.push(SortStructure::KNOWN_LIST { layout_type: LayoutType::TENXV2, max_distance: 1, on_disk: true, known_list: known_lists.get(&LayoutType::TENXV2).unwrap().clone() });
                ret.push(SortStructure::LD_UMI { layout_type: LayoutType::TENXV2, max_distance: 3, on_disk: false });
                ret
            }
            LayoutType::PAIREDUMI => { unimplemented!() }
            LayoutType::PAIRED => { unimplemented!() }
            LayoutType::SCI => { unimplemented!() }
            _ => { unimplemented!() }
        }
    }

    /// I couldn't get my sort structure to take a generic 'function parameter' to call for the correct sequences, so this
    /// method has to do that work in a less elegant way. Maybe I'll figure it out later
    ///
    pub fn get_known_list_value(&self, layout_type: &LayoutType, seq_layout: &dyn SequenceLayout, known_lists: HashMap<LayoutType, PathBuf>) -> PathBuf {
        match self {
            SortStructure::KNOWN_LIST { layout_type, max_distance: maximum_distance, on_disk, known_list } => {
                match layout_type {
                    LayoutType::TENXV3 => { known_lists.get(layout_type).unwrap().clone() }
                    LayoutType::TENXV2 => { unimplemented!() }
                    LayoutType::PAIREDUMI => { unimplemented!() }
                    LayoutType::PAIRED => { unimplemented!() }
                    LayoutType::SCI => { unimplemented!() }
                }
            }
            SortStructure::HD_UMI { layout_type, max_distance, on_disk } => {
                unimplemented!()
            }
            SortStructure::HD_UMI { layout_type, max_distance, on_disk } => {
                unimplemented!()
            }
            _ => { unimplemented!() }
        }
    }

    pub fn get_field(&self, seq_layout: &impl SequenceLayout) -> Option<Vec<u8>> {
        match self {
            SortStructure::KNOWN_LIST { layout_type, max_distance: maximum_distance, on_disk, known_list } => {
                match layout_type {
                    LayoutType::TENXV3 => { Some(seq_layout.cell_id().unwrap().clone()) }
                    LayoutType::TENXV2 => { Some(seq_layout.cell_id().unwrap().clone()) }
                    LayoutType::PAIREDUMI => { unimplemented!() }
                    LayoutType::PAIRED => { unimplemented!() }
                    LayoutType::SCI => { unimplemented!() }
                }
            }
            SortStructure::HD_UMI { layout_type, max_distance, on_disk } => {
                match layout_type {
                    LayoutType::TENXV3 => { Some(seq_layout.umi().unwrap().clone()) }
                    LayoutType::TENXV2 => { Some(seq_layout.umi().unwrap().clone()) }
                    LayoutType::PAIREDUMI => { unimplemented!() }
                    LayoutType::PAIRED => { unimplemented!() }
                    LayoutType::SCI => { unimplemented!() }
                }
            }
            SortStructure::LD_UMI { layout_type, max_distance, on_disk } => {
                match layout_type {
                    LayoutType::TENXV3 => { Some(seq_layout.umi().unwrap().clone()) }
                    LayoutType::TENXV2 => { Some(seq_layout.umi().unwrap().clone()) }
                    LayoutType::PAIREDUMI => { unimplemented!() }
                    LayoutType::PAIRED => { unimplemented!() }
                    LayoutType::SCI => { unimplemented!() }
                }
            }
        }
    }
}

pub struct Sorter {}

impl Sorter {
    pub fn sort(sort_list: Vec<SortStructure>,
                input_reads: &ReadFileContainer,
                tmp_location: &String,
                sorted_output: &String,
                layout: &LayoutType,
                run_specs: &mut RunSpecifications) -> Vec<ReadIterator> {
        let temp_location_base = Path::new(tmp_location);

        let mut read_iterator = ReadIterator::new_from_bundle(input_reads);

        let mut current_iterators = Vec::new();

        current_iterators.push(read_iterator);

        for sort in sort_list {
            println!("Sort level {}",sort.to_string());
            let mut next_level_iterators = Vec::new();

            for mut iter in current_iterators {
                let it = Sorter::sort_level(&sort, iter, layout, run_specs);
                match it {
                    None => {}
                    Some(x) => {
                        next_level_iterators.extend(x);
                    }
                }
            }
            current_iterators = next_level_iterators;

        }
        current_iterators
    }

    pub fn sort_level(sort_structure: &SortStructure, iterator: ReadIterator, layout: &LayoutType, run_specs: &mut RunSpecifications) -> Option<Vec<ReadIterator>> {
        match sort_structure {
            SortStructure::KNOWN_LIST { layout_type, max_distance: maximum_distance, on_disk, known_list } => {
                assert_eq!(*on_disk, true);
                let mut sorter = KnownListDiskStream::from_read_iterator(iterator, sort_structure, layout, run_specs);
                let mut read_sets = sorter.sorted_read_set();
                match read_sets {
                    None => None,
                    Some(x) => {
                        let mut ret = Vec::new();
                        for ci in x {
                            ret.push(ReadIterator::from_collection(ci));
                        }
                        Some(ret)
                    }
                }
            }
            SortStructure::HD_UMI { layout_type, max_distance, on_disk } |
            SortStructure::LD_UMI { layout_type, max_distance, on_disk } => {
                assert_eq!(*on_disk, false);
                let mut sorter = ClusteredDiskSortStream::from_read_iterator(iterator, sort_structure, layout, run_specs);
                let read_sets = sorter.sorted_read_set();
                match read_sets {
                    None => None,
                    Some(x) => {
                        let mut ret = Vec::new();
                        for ci in x {
                            ret.push(ReadIterator::from_collection(ci));
                        }
                        //println!("ITERRRRRATOR LENGTH {}", ret.len());
                        Some(ret)
                    }
                }
            }
        }
    }
}
