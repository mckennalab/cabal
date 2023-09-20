extern crate rust_spoa;

use std::cmp;
use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use rust_spoa::poa_consensus;
use shardio::{Range, ShardReader};
use crate::alignment::fasta_bit_encoding::{FASTA_UNSET, FastaBase};
use crate::read_strategies::read_disk_sorter::{SortingReadSetContainer};
use counter::Counter;
use indicatif::ProgressBar;
use rust_htslib::bam::record::{CigarString};
use crate::alignment::alignment_matrix::AlignmentTag;
use crate::alignment_functions::{create_sam_record, perform_rust_bio_alignment, setup_sam_writer, simplify_cigar_string};
use crate::reference::fasta_reference::ReferenceManager;
use rand::prelude::*;
use rust_htslib::bam::{Writer};


/*
#[derive(Message)]
#[rtype(result = "()")]
struct ReadBundle {
    reads: VecDeque<SortingReadSetContainer>,
    max_reads: usize,
}
#[derive(Message)]
#[rtype(result = "()")]
struct AlignedRead {
    read: Record
}

// Actor definition
struct ConsensusActor {
    reference: Vec<FastaBase>,
    reference_u8: Vec<u8>,
    writer: Arc<Mutex<Writer>>
}

impl Actor for ConsensusActor {
    type Context = Context<ConsensusActor>;
}
impl Handler<ReadBundle> for ConsensusActor {
    type Result = AlignedRead; // <- Message response type

    fn handle(&mut self, msg: ReadBundle, ctx: &mut Context<bool>) -> Self::Result {
        let read = self.output_buffered_read_set_to_sam_file(msg);
        ctx.recipient.do_send(AlignedRead { read })
    }
}

impl ConsensusActor {

    fn output_buffered_read_set_to_sam_file(&self,
                                            sg: ReadBundle) -> Record {

        let mut added_tags = HashMap::new();
        let mut buffered_reads = sg.reads.clone();
        added_tags.insert((b'r', b'c'), buffered_reads.len().to_string());
        added_tags.insert((b'd', b'c'), cmp::min(*sg.max_reads,buffered_reads.len()).to_string());

        let mut consensus_reads = create_poa_consensus(&buffered_reads, *sg.max_reads);
        let consensus_reference = Counter::<Vec<u8>, usize>::init(buffered_reads.iter().map(|x| x.aligned_read.ref_name.clone().as_bytes().to_vec()).collect::<Vec<Vec<u8>>>()).most_common_ordered();

        // TODO: this needs to be tied to their aligner choice!!!
        let new_alignment = perform_rust_bio_alignment(&self.reference, &FastaBase::from_vec_u8(&consensus_reads));
        let read_names = buffered_reads.iter().map(|x| x.aligned_read.read_name.clone()).collect::<Vec<String>>();

        added_tags.insert((b'a', b'r'), read_names.join(","));
        added_tags.insert((b'r', b'm'), get_reference_alignment_rate(&new_alignment.reference_aligned,&new_alignment.read_aligned).to_string());


        create_sam_record(read_names.get(0).clone().unwrap(),
                                         &new_alignment.read_aligned,
                                         &new_alignment.reference_aligned,
                                         &self.reference_u8,
                                         &reference_read_to_cigar_string(&new_alignment.reference_aligned, &new_alignment.read_aligned),
                                         &true,
                                         added_tags)
    }
}*/

pub fn write_consensus_reads(reader: &ShardReader<SortingReadSetContainer>,
                             output_file: &String,
                             levels: usize,
                             read_counts: &usize,
                             reference_manager: &ReferenceManager,
                             maximum_reads_before_downsampling: &usize) {

    info!("Writing consensus reads to {}", output_file);

    let mut writer = setup_sam_writer(output_file, reference_manager).unwrap();

    let mut last_read: Option<SortingReadSetContainer> = None;
    let mut buffered_reads = VecDeque::new();
    let bar = ProgressBar::new(read_counts.clone() as u64);

    reader.iter_range(&Range::all()).unwrap().for_each(|x| {
        bar.inc(1);
        let x = x.unwrap();
        assert_eq!(x.ordered_sorting_keys.len(), levels);
        if !(last_read.is_some() && &x.cmp(last_read.as_ref().unwrap()) == &Ordering::Equal) && buffered_reads.len() > 0 {
            output_buffered_read_set_to_sam_file(reference_manager, maximum_reads_before_downsampling, &mut writer, &mut buffered_reads);
            buffered_reads = VecDeque::new();
        }
        buffered_reads.push_back(x.clone());
        last_read = Some(x);
    });

    if buffered_reads.len() > 0 {
        output_buffered_read_set_to_sam_file(reference_manager, maximum_reads_before_downsampling, &mut writer, &mut buffered_reads);
    }
}

fn output_buffered_read_set_to_sam_file(reference_manager: &ReferenceManager,
                                        maximum_reads_before_downsampling: &usize,
                                        writer: &mut Writer,
                                        buffered_reads: &mut VecDeque<SortingReadSetContainer>) {
    let mut added_tags = HashMap::new();
    added_tags.insert((b'r', b'c'), buffered_reads.len().to_string());

    added_tags.insert((b'd', b'c'), cmp::min(*maximum_reads_before_downsampling,buffered_reads.len()).to_string());

    let consensus_reads = create_poa_consensus(&buffered_reads, maximum_reads_before_downsampling);
    let consensus_reference = Counter::<Vec<u8>, usize>::init(buffered_reads.iter().map(|x| x.aligned_read.ref_name.clone().as_bytes().to_vec()).collect::<Vec<Vec<u8>>>()).most_common_ordered();

    let top_ref = &consensus_reference.get(0).unwrap().clone().0;
    let reference_pointer = reference_manager.references.get(reference_manager.reference_name_to_ref.get(top_ref).unwrap()).unwrap();

    // TODO: this needs to be tied to their aligner choice!!!
    let new_alignment = perform_rust_bio_alignment(&reference_pointer.sequence, &FastaBase::from_vec_u8(&consensus_reads));
    let read_names = buffered_reads.iter().map(|x| x.aligned_read.read_name.clone()).collect::<Vec<String>>();

    added_tags.insert((b'a', b'r'), read_names.join(","));
    added_tags.insert((b'r', b'm'), get_reference_alignment_rate(&new_alignment.reference_aligned,&new_alignment.read_aligned).to_string());
    added_tags.insert((b'a', b's'), new_alignment.score.to_string());
    

    let sam_read = create_sam_record(read_names.get(0).clone().unwrap(),
                                     &new_alignment.read_aligned,
                                     &new_alignment.reference_aligned,
                                     &reference_pointer.sequence_u8,
                                     &reference_read_to_cigar_string(&new_alignment.reference_aligned, &new_alignment.read_aligned),
                                     &true,
                                     added_tags);

    writer.write(&sam_read).unwrap();
}

pub fn get_reference_alignment_rate(reference: &Vec<FastaBase>, read: &Vec<FastaBase>) -> f64 {
    let mut matches = 0;
    let mut mismatches = 0;

    for index in 0..reference.len() {
        if reference.get(index).unwrap() != &FASTA_UNSET && read.get(index).unwrap() != &FASTA_UNSET {
            if reference.get(index).unwrap() == read.get(index).unwrap()
            {
                matches += 1;
            } else {
                mismatches += 1;
            }
        }
    }

    (matches as f64) / ((matches + mismatches) as f64)
}
pub fn reference_read_to_cigar_string(reference_seq: &Vec<FastaBase>, read_seq: &Vec<FastaBase>) -> CigarString {
    let mut result: Vec<AlignmentTag> = Vec::new();

    for index in 0..reference_seq.len() {
        if *reference_seq.get(index).unwrap() == FASTA_UNSET {
            result.push(AlignmentTag::Ins(1))
        } else if *read_seq.get(index).unwrap() == FASTA_UNSET {
            result.push(AlignmentTag::Del(1))
        } else {
            result.push(AlignmentTag::MatchMismatch(1))
        }
    }

    let simplied_cigar = simplify_cigar_string(&result);
    CigarString::try_from(
        simplied_cigar.iter().map(|m| format!("{}", m)).collect::<Vec<String>>().join("").as_bytes()).
        expect("Unable to parse cigar string.")
}


pub fn create_poa_consensus(sequences: &VecDeque<SortingReadSetContainer>, downsample_to: &usize) -> Vec<u8> {
    let max_length = sequences.iter().map(|n| n.aligned_read.aligned_read.len()).collect::<Vec<usize>>();
    let max_length = max_length.iter().max().unwrap();

    let mut base_sequences = sequences.iter().map(|n| {
        let mut y = FastaBase::to_vec_u8(&n.aligned_read.aligned_read);
        y.push(b'\0');
        y
    }).collect::<Vec<Vec<u8>>>();

    // downsample if needed -- it's not the best idea to do downsampling here after all the work above,
    // but sometimes the borrow checker is a pain when your brain is small
    if base_sequences.len() > *downsample_to {
        let mut rng = rand::thread_rng();
        base_sequences = base_sequences.into_iter().choose_multiple(&mut rng, *downsample_to);
    }

    poa_consensus(&base_sequences, max_length.clone(), 1, 5, -4, -3, -1).
        iter().filter(|x| *x != &b'-').map(|x| *x).collect::<Vec<u8>>()
}