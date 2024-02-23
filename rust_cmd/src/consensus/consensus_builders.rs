extern crate spoa;

use std::cmp;
use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::ffi::CString;
use shardio::{Range, ShardReader};
use crate::alignment::fasta_bit_encoding::{FASTA_UNSET, FastaBase};
use crate::read_strategies::read_disk_sorter::SortingReadSetContainer;
use counter::Counter;
use spoa::{AlignmentEngine, Graph, AlignmentType};
use ndarray::Ix3;
use rust_htslib::bam::record::CigarString;
use crate::alignment::alignment_matrix::{Alignment, AlignmentTag, AlignmentType as LocalAlignmentType, create_scoring_record_3d};
use crate::reference::fasta_reference::ReferenceManager;
use rand::prelude::*;
use crate::alignment::scoring_functions::{AffineScoring};
use crate::alignment_manager::{align_two_strings, OutputAlignmentWriter, simplify_cigar_string};

pub fn write_consensus_reads(reader: &ShardReader<SortingReadSetContainer>,
                             writer: &mut dyn OutputAlignmentWriter,
                             levels: usize,
                             reference_manager: &ReferenceManager,
                             maximum_reads_before_downsampling: &usize) {

    let mut last_read: Option<SortingReadSetContainer> = None;
    let mut buffered_reads = VecDeque::new();

    let mut written_buffers = 0;
    let mut processed_reads = 0;

    let score = AffineScoring::default();
    let mut alignment_mat: Alignment<Ix3> = create_scoring_record_3d((reference_manager.longest_ref + 1) * 2, (reference_manager.longest_ref + 1) * 2, LocalAlignmentType::Affine, false);


    reader.iter_range(&Range::all()).unwrap().for_each(|x| {
        let x = x.unwrap();
        assert_eq!(x.ordered_sorting_keys.len(), levels);
        if !(last_read.is_some() && x.cmp(last_read.as_ref().unwrap()) == Ordering::Equal) && !buffered_reads.is_empty() {
            output_buffered_read_set_to_sam_file(reference_manager,  maximum_reads_before_downsampling, writer, &buffered_reads, &score, &mut alignment_mat);
            written_buffers += 1;
            buffered_reads = VecDeque::new();
        }
        processed_reads += 1;
        buffered_reads.push_back(x.clone());
        last_read = Some(x);
    });

    if !buffered_reads.is_empty() {
        output_buffered_read_set_to_sam_file(reference_manager, maximum_reads_before_downsampling, writer, &buffered_reads, &score, &mut alignment_mat);
        written_buffers += 1;
    }
    info!("Processed {} reads into {} consensus reads", processed_reads, written_buffers);
}

fn output_buffered_read_set_to_sam_file(reference_manager: &ReferenceManager,
                                        maximum_reads_before_downsampling: &usize,
                                        writer: &mut dyn OutputAlignmentWriter,
                                        buffered_reads: &VecDeque<SortingReadSetContainer>,
                                        my_aff_score: &AffineScoring,
                                        mut alignment_mat: &mut Alignment<Ix3>) {
    let mut added_tags = HashMap::new();
    added_tags.insert((b'r', b'c'), buffered_reads.len().to_string());
    added_tags.insert((b'd', b'c'), cmp::min(*maximum_reads_before_downsampling, buffered_reads.len()).to_string());


    if buffered_reads.len() > 1 {
        let consensus_reads = create_poa_consensus(buffered_reads, maximum_reads_before_downsampling);
        let consensus_reference = Counter::<Vec<u8>, usize>::init(buffered_reads.iter().map(|x| x.aligned_read.reference_name.clone().as_bytes().to_vec()).collect::<Vec<Vec<u8>>>()).most_common_ordered();

        let top_ref = &consensus_reference.get(0).unwrap().clone().0;
        let reference_pointer = reference_manager.references.get(reference_manager.reference_name_to_ref.get(top_ref).unwrap()).unwrap();
        let new_alignment = align_two_strings(&FastaBase::from_vec_u8(&consensus_reads), &reference_pointer.sequence, my_aff_score, false, None, None);

        let read_names = buffered_reads.iter().map(|x| x.aligned_read.read_name.clone()).collect::<Vec<String>>();

        added_tags.insert((b'a', b'r'), read_names.join(","));
        added_tags.insert((b'r', b'm'), get_reference_alignment_rate(&new_alignment.reference_aligned, &new_alignment.read_aligned).to_string());
        added_tags.insert((b'a', b's'), new_alignment.score.to_string());
        let new_sorting_read = buffered_reads.get(0).unwrap().with_new_alignment(new_alignment);
        
        writer.write_read(&new_sorting_read,&added_tags);

    } else {
        let single_read = buffered_reads.get(0).unwrap();
        added_tags.insert((b'a', b'r'), single_read.aligned_read.read_name.clone());
        added_tags.insert((b'r', b'm'), get_reference_alignment_rate(&single_read.aligned_read.reference_aligned,
                                                                     &single_read.aligned_read.read_aligned).to_string());
        added_tags.insert((b'a', b's'), single_read.aligned_read.score.to_string());

        writer.write_read(&single_read,&added_tags).unwrap();
    };
}

pub fn get_reference_alignment_rate(reference: &Vec<FastaBase>, read: &[FastaBase]) -> f64 {
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

pub fn reference_read_to_cigar_string(reference_seq: &Vec<FastaBase>, read_seq: &[FastaBase]) -> CigarString {
    let mut result: Vec<AlignmentTag> = Vec::new();

    for index in 0..reference_seq.len() {
        if reference_seq.get(index).unwrap().eq(&FASTA_UNSET) {
            result.push(AlignmentTag::Ins(1))
        } else if read_seq.get(index).unwrap().eq(&FASTA_UNSET) {
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
    let mut base_sequences = sequences.iter().map(|n| {
        let mut y = FastaBase::vec_u8(&FastaBase::strip_gaps(&n.aligned_read.read_aligned));
        y.push(b'\0');
        y
    }).collect::<Vec<Vec<u8>>>();

    // downsample if needed -- it's not the best idea to do downsampling here after all the work above,
    // but sometimes the borrow checker is a pain when your brain is small
    if base_sequences.len() > *downsample_to {
        let mut rng = thread_rng();
        base_sequences = base_sequences.into_iter().choose_multiple(&mut rng, *downsample_to);
    }
    poa_consensus(base_sequences)
}

fn poa_consensus(base_sequences: Vec<Vec<u8>>) -> Vec<u8> {
    let mut eng = AlignmentEngine::new(AlignmentType::kNW, 5, -4, -3, -1, -3, -1);
    let mut graph = Graph::new();

    for seq in &base_sequences {
        assert!(!seq.is_empty());

        let cseq = CString::from_vec_with_nul(seq.clone()).unwrap_or_else(|_| panic!("CString::new failed from {}", String::from_utf8(seq.clone()).unwrap()));
        let cqual = CString::new(vec![34u8; seq.len() - 1]).unwrap();

        let aln = eng.align(cseq.as_c_str(), &graph);
        graph.add_alignment(&aln, cseq.as_c_str(), &cqual);
    }

    graph.consensus().to_str().unwrap().to_owned().into_bytes()
}


// reference_read_to_cigar_string

#[cfg(test)]
mod tests {
    use rust_htslib::bam::record::Cigar;
    use super::*;

    #[test]
    fn test_cigar_string() {
        let reference = FastaBase::from_string(&"CGTACGCTAGACATTGTGCCGCATCGATTGTAGTGACAATAGGAAA-------TATACAAG".to_string());
        let read = FastaBase::from_string(&"CGT-----AGACATTGTGCCGCATCGATTGTAGTGACAATAGGAAATGACGGCTATACAAG".to_string());
        let cigar = reference_read_to_cigar_string(&reference, &read);
        let true_cigar = CigarString(vec![Cigar::Match(3), Cigar::Del(5), Cigar::Match(38), Cigar::Ins(7), Cigar::Match(8)]);
        assert_eq!(cigar, true_cigar);
        // CGTACGCTAGACATTGTGCCGCATCGATTGTAGTGACAATAGGAAATGACGGCTATACAAG-----------------------------------------------------------------------------------------------------------------------------------------------------------------GTAGGAGCCGTTACCAGGATGA------AGGTTATTAGGGGATCCGCTTTAAGGCCGGTCCTAGCAACAAGCTAACGGTGCAGGATCTTGGGTTTCTGTCTCTTATTCACATCTGACGCTGCCGACGACGAGGTATAAGTGTAGATATCGGTGGTCGCCGTATCATT
        // AAACCCAAGATCCTGCACCGTTAGCTTGCGTACGCTAGACATTGTGCCGCATCGATTGTAGTGACAATAGGAAATGACGGCTATACAAGGTAGGAGCCGTTACCAGGATGAAGGTTATTAGGGGATCCGCTTTAAGGCCGGTCCTAGCAACAAGCTAACGGTGCAGGATCTTGGGTTTCTGTCTCTTATTCACATCTGACGCTGCCGACGACGAGGTATAAGTGTAGATATCGGTGGTCGCCGTATCATTACAAAAGTGGGTGGGGGGGGGGGGGGGGC
        // CGTACGCTAGACATTGTGCCGCATC22222222222222TAGGAAATGACGGCTATACAAGGCATCGCGGTGTCTCGTCAATACACCTTACGGAGGCATTGGATGATAATGTCGCAAGGAGGTCTCAAGATTCTGTACCACACGTCGGCACGCGATTGAACCAATGGACAGAGGACAGGATACGTAGGATCACCAACTAGGTCATTAGGTGGAAGGTGATACGTAGGAGCCGTTACCAGGATGAACGATGAGGTTATTAGGGGATCCGCTTTAAGGCCGGTCCTAGCAANNNNNNNNNNNNNNNNNNNNNNNNNNNNCTGTCTCTTATACACATCTGACGCTGCCGACGANNNNNNNNNNGTGTAGATCTCGGTGGTCGCCGTATCATT
    }

    #[test]
    fn test_consensus_string() {
        let read1 = "ACGTACGT\0".as_bytes().to_vec();
        let read2 = "ACGTACGT\0".as_bytes().to_vec();
        let read3 = "ACGTAC-T\0".as_bytes().to_vec();
        let vec_of_reads = vec![read1, read2, read3];
        let result = poa_consensus(vec_of_reads);
        assert_eq!(result, "ACGTACGT".as_bytes().to_vec());

        let read1 = "ACGTACGT\0".as_bytes().to_vec();
        let read2 = "ACGTAC-T\0".as_bytes().to_vec();
        let read3 = "ACGTAC-T\0".as_bytes().to_vec();
        let vec_of_reads = vec![read1, read2, read3];
        let result = poa_consensus(vec_of_reads);
        assert_eq!(result, "ACGTAC-T".as_bytes().to_vec());

        let read1 = "ACGTACGT\0".as_bytes().to_vec();
        let read2 = "AAAAAACGTAC-T\0".as_bytes().to_vec();
        let read3 = "ACGTAC-T\0".as_bytes().to_vec();
        let vec_of_reads = vec![read1, read2, read3];
        let result = poa_consensus(vec_of_reads);
        assert_eq!(result, "AAAAAACGTAC-T".as_bytes().to_vec());

        let read1 = "ACGTACGTTTT\0".as_bytes().to_vec();
        let read2 = "AAAAAACGTAC-TTTT\0".as_bytes().to_vec();
        let read3 = "ACGTAC-T\0".as_bytes().to_vec();
        let vec_of_reads = vec![read1, read2, read3];
        let result = poa_consensus(vec_of_reads);
        assert_eq!(result, "AAAAAACGTAC-TTTT".as_bytes().to_vec());
    }
}