use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str;
use crate::itertools::Itertools;
use crate::rayon::iter::ParallelBridge;
use crate::rayon::iter::ParallelIterator;
use crate::alignment::alignment_matrix::{Alignment, AlignmentResult, AlignmentTag, AlignmentType, create_scoring_record_3d, perform_3d_global_traceback, perform_affine_alignment};
use crate::alignment::scoring_functions::{AffineScoring, AffineScoringFunction, InversionScoring};
use crate::extractor::{extract_tagged_sequences, gap_proportion_per_tag, READ_CHAR, REFERENCE_CHAR, stretch_sequence_to_alignment};
use crate::linked_alignment::{orient_by_longest_segment};
use crate::read_strategies::read_set::{ReadIterator};
use crate::reference::fasta_reference::{Reference, ReferenceManager};
use std::time::{Instant};
use bio::alignment::AlignmentOperation;
use ndarray::Ix3;
use shardio::{ShardReader, ShardWriter};
use crate::alignment::fasta_bit_encoding::{FASTA_UNSET, FastaBase, reverse_complement};
use crate::merger::{MergedReadSequence};
use crate::read_strategies::sequence_layout::{SequenceLayoutDesign, UMIConfiguration};
use libwfa::{affine_wavefront::*, bindings::*, mm_allocator::*, penalties::*};
use rust_htslib::bam;
use rust_htslib::bam::{Record};
use rust_htslib::bam::record::{Aux, CigarString};
use crate::read_strategies::read_disk_sorter::{SortedAlignment, SortingReadSetContainer};
use bio::alignment::pairwise::Aligner;


pub fn align_reads(use_capture_sequences: &bool,
                   read_structure: &SequenceLayoutDesign,
                   only_output_captured_ref: &bool,
                   to_fake_fastq: &bool,
                   rm: &ReferenceManager,
                   output: &Path,
                   max_reference_multiplier: &usize,
                   min_read_length: &usize,
                   read1: &String,
                   read2: &String,
                   index1: &String,
                   index2: &String,
                   threads: &usize,
                   inversions: &bool) {
    let output_file = File::create(&output).unwrap();

    let read_iterator = ReadIterator::new(PathBuf::from(&read1),
                                          Some(PathBuf::from(&read2)),
                                          Some(PathBuf::from(&index1)),
                                          Some(PathBuf::from(&index2)));

    let read_iterator = MergedReadSequence::new(read_iterator, read_structure);

    let output = Arc::new(Mutex::new(output_file)); // Mutex::new(gz));

    // setup our thread pool
    rayon::ThreadPoolBuilder::new().num_threads(*threads).build_global().unwrap();

    let my_score = InversionScoring {
        match_score: 9.0,
        mismatch_score: -21.0,
        gap_open: -25.0,
        gap_extend: -1.0,
        inversion_penalty: -40.0,
        min_inversion_length: 20,
    };

    let my_aff_score = AffineScoring {
        match_score: 10.0,
        mismatch_score: -9.0,
        special_character_score: 9.0,
        gap_open: -20.0,
        gap_extend: -1.0,
        final_gap_multiplier: 1.0,

    };
    let start = Instant::now();
    let read_count = Arc::new(Mutex::new(0)); // we rely on this Arc for output file access control

    type SharedStore = Arc<Mutex<Option<Alignment<Ix3>>>>;

    lazy_static! {
        static ref STORE_CLONES: Mutex<Vec<SharedStore>> = Mutex::new(Vec::new());
    }
    thread_local!(static STORE: SharedStore = Arc::new(Mutex::new(None)));

    let alignment_mat: Alignment<Ix3> = create_scoring_record_3d((rm.longest_ref + 1) * 2, (rm.longest_ref + 1) * 2, AlignmentType::AFFINE, false);

    read_iterator.par_bridge().for_each(|xx| {
        STORE.with(|arc_mtx| {
            let mut local_alignment = arc_mtx.lock().unwrap();
            if local_alignment.is_none() {
                *local_alignment = Some(alignment_mat.clone());
                STORE_CLONES.lock().unwrap().push(arc_mtx.clone());
            }

            let name = &String::from_utf8(xx.name).unwrap();

            let aligned = best_reference(&xx.seq,
                                         &rm.references,
                                         read_structure,
                                         &mut local_alignment.as_mut().unwrap(),
                                         &my_aff_score,
                                         &my_score,
                                         inversions,
                                         *max_reference_multiplier,
                                         *min_read_length);
            //println!("Aligning read {}", FastaBase::to_string(&xx.seq));
            match aligned {
                None => {
                    warn!("Unable to find alignment for read {}",name);
                }
                Some((results, orig_ref_seq, ref_name)) => {
                    match results {
                        None => { warn!("Unable to find alignment for read {}",name); }
                        Some(aln) => {
                            let output = Arc::clone(&output);
                            let mut read_count = read_count.lock().unwrap();
                            *read_count += 1;
                            if *read_count % 1000000 == 0 {
                                let duration = start.elapsed();
                                info!("Time elapsed in aligning reads ({:?}) is: {:?}", read_count, duration);
                            }
                            assert_eq!(aln.reference_aligned.len(), aln.read_aligned.len());
                            let cigar_string = format!("{}", "");


                            output_alignment(&FastaBase::to_vec_u8(&aln.read_aligned),
                                             name,
                                             &FastaBase::to_vec_u8(&aln.reference_aligned),
                                             &orig_ref_seq,
                                             &ref_name,
                                             use_capture_sequences,
                                             only_output_captured_ref,
                                             to_fake_fastq,
                                             output,
                                             &cigar_string,
                                             min_read_length);
                        }
                    }
                }
            }
        });
    });
}

pub fn fast_align_reads(_use_capture_sequences: &bool,
                        read_structure: &SequenceLayoutDesign,
                        _only_output_captured_ref: &bool,
                        _to_fake_fastq: &bool,
                        rm: &ReferenceManager,
                        output: &Path,
                        max_reference_multiplier: &f64,
                        _min_read_length: &usize,
                        read1: &String,
                        read2: &String,
                        index1: &String,
                        index2: &String,
                        max_gaps_proportion: &f64,
                        threads: &usize) -> (usize, ShardReader<SortingReadSetContainer>) {

    let read_iterator = ReadIterator::new(PathBuf::from(&read1),
                                          Some(PathBuf::from(&read2)),
                                          Some(PathBuf::from(&index1)),
                                          Some(PathBuf::from(&index2)));

    let read_iterator = MergedReadSequence::new(read_iterator, read_structure);

    let mut sharded_output: ShardWriter<SortingReadSetContainer> = ShardWriter::new(&output, 32,
                                                                                    256,
                                                                                    1 << 16).unwrap();

    let sender = Arc::new(Mutex::new(sharded_output.get_sender()));

    let mut sorting_order = read_structure.umi_configurations.iter().map(|x| x.1.clone()).collect::<Vec<UMIConfiguration>>();
    sorting_order.sort_by(|a, b| a.order.cmp(&b.order));
    let sorted_tags = sorting_order.iter().map(|x| x.symbol).collect::<Vec<char>>();

    // setup our thread pool
    rayon::ThreadPoolBuilder::new().num_threads(*threads).build_global().unwrap();

    let start = Instant::now();

    assert_eq!(rm.references.len(), 1);
    let reference_seq = rm.references.iter().next().unwrap();
    let reference_bases = FastaBase::from_vec_u8_default_ns(&reference_seq.1.sequence_u8);
    let reference_name = String::from_utf8(reference_seq.1.name.clone()).unwrap();
    let read_count = Arc::new(Mutex::new(0 as usize));
    let skipped_count = Arc::new(Mutex::new(0 as usize));
    let gap_rejected = Arc::new(Mutex::new(0 as usize));

    read_iterator.par_bridge().for_each(|xx| {
        if (xx.seq.len() as f64) < ((*max_reference_multiplier) * reference_bases.len() as f64) {
            let forward_oriented_seq = if !read_structure.known_orientation {
                let orientation = orient_by_longest_segment(&xx.seq, &reference_seq.1.sequence_u8, &reference_seq.1.suffix_table).0;
                if orientation {
                    xx.seq.clone()
                } else {
                    reverse_complement(&xx.seq)
                }
            } else {
                xx.seq.clone()
            };
            let alignment = align_using_selected_aligner(read_structure, &reference_bases,&forward_oriented_seq);

            let ref_al = FastaBase::to_vec_u8(&alignment.reference_aligned);
            let read_al = FastaBase::to_vec_u8(&alignment.read_aligned);
            let full_ref = stretch_sequence_to_alignment(&ref_al, &reference_seq.1.sequence_u8);
            let ets = extract_tagged_sequences(&read_al, &full_ref);

            let gap_proportion = gap_proportion_per_tag(&ets);
            //println!("Gap proportion: {:?} {:?} from {} tested {} and {}", ets, gap_proportion, gap_proportion.iter().max_by(|a, b| a.total_cmp(b)).unwrap(), gap_proportion.iter().max_by(|a, b| a.total_cmp(b)).unwrap() <= max_gaps_proportion, *max_gaps_proportion);
            if gap_proportion.iter().max_by(|a, b| a.total_cmp(b)).unwrap() <= max_gaps_proportion {
                let read_tags_ordered = VecDeque::from(sorted_tags.iter().
                    map(|x| (x.clone(), ets.get(&x.to_string().as_bytes()[0]).unwrap().as_bytes().iter().map(|f| FastaBase::from(f.clone())).collect::<Vec<FastaBase>>())).collect::<Vec<(char, Vec<FastaBase>)>>());

                let new_sorted_read_container = SortingReadSetContainer {
                    ordered_sorting_keys: vec![],
                    ordered_unsorted_keys: read_tags_ordered,

                    aligned_read: SortedAlignment {
                        aligned_read: alignment.read_aligned.clone(),
                        aligned_ref: alignment.reference_aligned,
                        ref_name: reference_name.clone(),
                        read_name: String::from_utf8(xx.name.clone()).unwrap(),
                    },
                };
                assert_eq!(new_sorted_read_container.ordered_unsorted_keys.len(), read_structure.umi_configurations.len());

                let sender = Arc::clone(&sender);

                sender.lock().unwrap().send(new_sorted_read_container).unwrap();
            } else {
                *gap_rejected.lock().unwrap() += 1;
            }
        } else {
            *skipped_count.lock().unwrap() += 1;
        }
        *read_count.lock().unwrap() += 1;
        if *read_count.lock().unwrap() % 1000000 == 0 {
            let duration = start.elapsed();
            println!("Time elapsed in aligning reads ({:?}) is: {:?}", read_count.lock().unwrap(), duration);
        }
    });

    warn!("Skipped {} reads since their length was greater than {}, which is {} times the reference length; skipped {} reads for containing too many gaps in tags, processed {} reads",
        skipped_count.lock().unwrap(),
        (*max_reference_multiplier) * reference_bases.len() as f64,
        max_reference_multiplier,
        *gap_rejected.lock().unwrap(),
        *read_count.lock().unwrap());
    sender.lock().unwrap().finished().unwrap();
    sharded_output.finish().unwrap();

    let final_count = read_count.lock().unwrap().clone() - skipped_count.lock().unwrap().clone();
    (final_count, ShardReader::open(output).unwrap())
}

pub fn align_using_selected_aligner(read_structure: &SequenceLayoutDesign,
                                    reference_bases: &Vec<FastaBase>,
                                    xx: &Vec<FastaBase>) -> AlignmentResult {
    match &read_structure.aligner {
        None => {
            perform_rust_bio_alignment(&reference_bases, &xx) // current default
        }
        Some(x) => {
            match x.as_str() {
                "wavefront" => {perform_wavefront_alignment(&reference_bases, &xx)}
                "rustbio" => {perform_rust_bio_alignment(&reference_bases, &xx)}
                //"inversion_aware" => {perform_inversion_aware_alignment(&reference_bases, &xx.seq)}
                //"degenerate" => {perform_degenerate_alignment(&reference_bases, &xx.seq)}
                _ => {panic!("Unknown alignment method {}", x)}
            }
        }
    }
}

#[allow(dead_code)]
pub fn align_two_strings(read1_seq: &Vec<FastaBase>, rev_comp_read2: &Vec<FastaBase>, scoring_function: &dyn AffineScoringFunction, local: bool) -> AlignmentResult {
    let mut alignment_mat = create_scoring_record_3d(
        read1_seq.len() + 1,
        rev_comp_read2.len() + 1,
        AlignmentType::AFFINE,
        local);

    perform_affine_alignment(
        &mut alignment_mat,
        read1_seq,
        rev_comp_read2,
        scoring_function);

    perform_3d_global_traceback(
        &mut alignment_mat,
        None,
        read1_seq,
        rev_comp_read2,
        None)
}

pub fn best_reference(read: &Vec<FastaBase>,
                      references: &HashMap<usize, Reference>,
                      read_structure: &SequenceLayoutDesign,
                      alignment_mat: &mut Alignment<Ix3>,
                      my_aff_score: &AffineScoring,
                      my_score: &InversionScoring,
                      use_inversions: &bool,
                      max_reference_multiplier: usize,
                      min_read_length: usize) -> Option<(Option<AlignmentResult>, Vec<u8>, Vec<u8>)> {
    match references.len() {
        0 => {
            warn!("Unable to align read {} as it has no candidate references",FastaBase::to_string(read));
            None
        }
        1 => {
            let aln = alignment(read, &references.get(&0).unwrap(), read_structure, alignment_mat, my_aff_score, my_score, use_inversions, max_reference_multiplier, min_read_length);
            Some((aln, references.get(&0).unwrap().sequence_u8.clone(), references.get(&0).unwrap().name.clone()))
        }
        x if x > 1 => {
            let ranked_alignments = references.iter().map(|reference| {
                match alignment(read, reference.1, read_structure, alignment_mat, my_aff_score, my_score, use_inversions, max_reference_multiplier, min_read_length) {
                    None => None,
                    Some(aln) => Some((aln, reference.1.sequence_u8.clone(), reference.1.name.clone())),
                }
            }).filter(|x| x.is_some()).map(|c| c.unwrap());

            let ranked_alignments = ranked_alignments.into_iter().enumerate().max_by(|al, al2|
                matching_read_bases_prop(&al.1.0.read_aligned, &al.1.0.reference_aligned).
                    partial_cmp(&matching_read_bases_prop(&al2.1.0.read_aligned, &al2.1.0.reference_aligned)).unwrap());

            match ranked_alignments.iter().next() {
                None => { None }
                Some((_x, y)) => {
                    Some((Some(y.0.clone()), y.1.clone(), y.2.clone()))
                }
            }
        }
        x => { panic!("we dont know what to do with a reference count of {}", x) }
    }
}

// TODO bring back inversions
pub fn alignment(x: &Vec<FastaBase>,
                 reference: &Reference,
                 read_structure: &SequenceLayoutDesign,
                 _alignment_mat: &mut Alignment<Ix3>,
                 _my_aff_score: &AffineScoring,
                 _my_score: &InversionScoring,
                 _use_inversions: &bool,
                 max_reference_multiplier: usize,
                 min_read_length: usize) -> Option<AlignmentResult> {
    let forward_oriented_seq = if !read_structure.known_orientation {
        let orientation = orient_by_longest_segment(x, &reference.sequence_u8, &reference.suffix_table).0;
        if orientation {
            x.clone()
        } else {
            reverse_complement(&x)
        }
    } else {
        x.clone()
    };

    if forward_oriented_seq.len() > reference.sequence.len() * max_reference_multiplier || forward_oriented_seq.len() < min_read_length {
        warn!("Dropping read of length {}",forward_oriented_seq.len());
        None
    } else {
        /*perform_affine_alignment(
            alignment_mat,
            &reference.sequence,
            &forward_oriented_seq,
            my_aff_score);

        let results = perform_3d_global_traceback(
            alignment_mat,
            None,
            &reference.sequence,
            &forward_oriented_seq,
            None);
        Some(results)*/
        // TODO: allow the users to pick which aligner to use
        Some(perform_wavefront_alignment(&reference.sequence,
                                         &forward_oriented_seq))
    }
}

fn cigar_to_alignment(reference: &Vec<FastaBase>,
                      read: &Vec<FastaBase>,
                      cigar: &Vec<u8>) -> (Vec<FastaBase>, Vec<FastaBase>) {
    let mut alignment_string1 = Vec::new();
    let mut alignment_string2 = Vec::new();
    let mut seq1_index = 0;
    let mut seq2_index = 0;

    for c in cigar {
        match c {
            b'M' | b'X' => {
                alignment_string1.push(reference.get(seq1_index).unwrap().clone());
                alignment_string2.push(read.get(seq2_index).unwrap().clone());
                seq1_index += 1;
                seq2_index += 1;
            }
            b'D' => {
                alignment_string1.push(reference.get(seq1_index).unwrap().clone());
                alignment_string2.push(FASTA_UNSET);
                seq1_index += 1;
            }
            b'I' => {
                alignment_string1.push(FASTA_UNSET);
                alignment_string2.push(read.get(seq2_index).unwrap().clone());
                seq2_index += 1;
            }
            _ => { panic!("Unknown cigar operation {}", c) }
        }
    };
    (alignment_string1, alignment_string2)
}

pub fn perform_rust_bio_alignment(reference: &Vec<FastaBase>, read: &Vec<FastaBase>) -> AlignmentResult {
    let score = |a: u8, b: u8| if a == b || a == 'N' as u8 || b == 'N' as u8 { 1i32 } else { -1i32 };

    let mut aligner = Aligner::new(-10, -1, &score);
    let x = FastaBase::to_vec_u8(&reference);
    let y = FastaBase::to_vec_u8(&read);
    let alignment = aligner.global(y.as_slice(), x.as_slice());
    bio_to_alignment_result(alignment, reference, read)
}

pub fn bio_to_alignment_result(alignment: bio::alignment::Alignment, reference: &Vec<FastaBase>, read: &Vec<FastaBase>) -> AlignmentResult {
    let mut aligned_ref = Vec::new();
    let mut aligned_read = Vec::new();
    let mut ref_pos = alignment.xstart;
    let mut read_pos = alignment.ystart;
    let mut resulting_cigar = Vec::new();
    for al in alignment.operations {
        match al {
            AlignmentOperation::Match => {
                aligned_ref.push(reference.get(ref_pos).unwrap().clone());
                aligned_read.push(read.get(read_pos).unwrap().clone());
                ref_pos += 1;
                read_pos += 1;
                resulting_cigar.push(AlignmentTag::MatchMismatch(1));
            }
            AlignmentOperation::Subst => {
                aligned_ref.push(reference.get(ref_pos).unwrap().clone());
                aligned_read.push(read.get(read_pos).unwrap().clone());
                ref_pos += 1;
                read_pos += 1;
                resulting_cigar.push(AlignmentTag::MatchMismatch(1));

            }
            AlignmentOperation::Del => {
                aligned_ref.push(reference.get(ref_pos).unwrap().clone());
                aligned_read.push(FASTA_UNSET);
                ref_pos += 1;
                resulting_cigar.push(AlignmentTag::Del(1));

            }
            AlignmentOperation::Ins => {
                aligned_ref.push(FASTA_UNSET);
                aligned_read.push(read.get(read_pos).unwrap().clone());
                read_pos += 1;
                resulting_cigar.push(AlignmentTag::Ins(1));

            }
            AlignmentOperation::Xclip(x) => {
                aligned_ref.extend(reference[ref_pos..ref_pos + x].iter());
                aligned_read.extend(FastaBase::from_vec_u8(&vec![b'-';x]).iter());
                ref_pos += x.clone();
                resulting_cigar.push(AlignmentTag::Ins(x));

            }
            AlignmentOperation::Yclip(y) => {
                aligned_read.extend(read[read_pos..read_pos + y].iter());
                aligned_ref.extend(FastaBase::from_vec_u8(&vec![b'-';y]).iter());
                read_pos += y.clone();
                resulting_cigar.push(AlignmentTag::Del(y));

            }
        }
    }
    AlignmentResult{
        reference_aligned: aligned_ref,
        read_aligned: aligned_read,
        cigar_string: simplify_cigar_string(&resulting_cigar),
        path: vec![],
        score: alignment.score as f64,
        bounding_box: None,
    }
}

pub fn perform_wavefront_alignment(reference: &Vec<FastaBase>, read: &Vec<FastaBase>) -> AlignmentResult {
    let alloc = MMAllocator::new(BUFFER_SIZE_8M as u64);

    let mut penalties = AffinePenalties {
        match_: 0,
        mismatch: 4,
        gap_opening: 6,
        gap_extension: 2,
    };

    let pat_len = read.len();
    let text_len = reference.len();

    let mut wavefronts = AffineWavefronts::new_complete(
        text_len,
        pat_len,
        &mut penalties,
        &alloc,
    );

    match wavefronts
        .align(FastaBase::to_vec_u8(reference).as_slice(), FastaBase::to_vec_u8(read).as_slice()) {
        Ok(_) => {}
        Err(x) => { panic!("Failed to align reference: {} and read: {} with error {:?}", FastaBase::to_string(&reference), FastaBase::to_string(&read), x) }
    }

    let cigar = wavefronts.cigar_bytes_raw();
    let score = wavefronts.edit_cigar_score(&mut penalties);
    let ref_read = cigar_to_alignment(reference, read, &cigar);


    AlignmentResult {
        reference_aligned: ref_read.0,
        read_aligned: ref_read.1,
        cigar_string: cigar.iter().map(|x| AlignmentTag::from(x.clone())).collect::<Vec<AlignmentTag>>(),
        path: vec![],
        score: score as f64,
        bounding_box: None,
    }
}


pub fn matching_read_bases_prop(read: &Vec<FastaBase>, reference: &Vec<FastaBase>) -> f32 {
    assert_eq!(read.len(), reference.len());
    let mut total_read_bases = 0;
    let mut matched = 0;
    read.iter().zip(reference.iter()).for_each(|(readb, refb)| {
        if *readb != FASTA_UNSET {
            total_read_bases += 1;
        }
        if *readb != FASTA_UNSET && readb == refb {
            matched += 1;
        }
    });
    if total_read_bases == 0 {
        0.0
    } else {
        matched as f32 / total_read_bases as f32
    }
}

pub fn tags_to_output(tags: &BTreeMap<u8, String>) -> String {
    tags.iter().filter(|(k, _v)| **k != READ_CHAR && **k != REFERENCE_CHAR).map(|(k, v)| format!("key={}:{}", String::from_utf8(vec![*k]).unwrap(), v)).join(";")
}

pub fn setup_sam_writer(filename: &String, reference_manger: &ReferenceManager) -> Result<bam::Writer, rust_htslib::errors::Error> {
    let mut header = bam::Header::new();
    reference_manger.references.iter().for_each(|reference| {
        let mut header_record = bam::header::HeaderRecord::new(b"SQ");
        header_record.push_tag(b"SN", String::from_utf8(reference.1.name.clone()).unwrap());
        header_record.push_tag(b"LN", &reference.1.sequence.len());
        header.push_record(&header_record);
    });

    bam::Writer::from_path(&filename, &header, bam::Format::Bam)
}


pub fn create_sam_record(
    read_name: &str,
    read_seq: &Vec<FastaBase>,
    reference_aligned_seq: &Vec<FastaBase>,
    reference_original_seq: &Vec<u8>,
    cigar_string: &CigarString,
    extract_capture_tags: &bool,
    additional_tags: HashMap<(u8, u8), String>) -> Record {
    let mut record = Record::new();
    let seq = FastaBase::to_vec_u8(
        &read_seq.clone().iter().cloned().filter(|b| *b != FASTA_UNSET).collect::<Vec<FastaBase>>());

    // we don't currently calculate real quality scores
    let quals = vec![255 as u8; seq.len()];

    record.set(read_name.as_bytes(), Some(&cigar_string), &seq.as_slice(), &quals.as_slice());

    additional_tags.iter().for_each(|(x, y)| {
        record.push_aux(vec![x.0, x.1].as_slice(), Aux::String(y)).unwrap();
    });


    if *extract_capture_tags {
        let full_ref = stretch_sequence_to_alignment(&FastaBase::to_vec_u8(&reference_aligned_seq), reference_original_seq);
        let ets = extract_tagged_sequences(&FastaBase::to_vec_u8(&read_seq), &full_ref);

        ets.iter().for_each(|(tag, seq)| {
            record.push_aux(vec![b'e', (*tag)].as_slice(), Aux::String(seq)).unwrap();
        });
    }
    record
}

/// Handle output of the alignment results based on the requested output formats
///
pub fn output_alignment(aligned_read: &Vec<u8>,
                        aligned_name: &String,
                        aligned_ref: &Vec<u8>,
                        original_ref: &Vec<u8>,
                        reference_name: &Vec<u8>,
                        use_capture_sequences: &bool,
                        only_output_captured_ref: &bool,
                        to_fake_fastq: &bool,
                        output: Arc<Mutex<File>>,
                        _cigar_string: &String,
                        min_read_length: &usize) {
    let mut output = output.lock().unwrap();

    let (ref_seq, read_seq, read_tags) =
        if *use_capture_sequences {
            let full_ref = stretch_sequence_to_alignment(aligned_ref, original_ref);
            let ets = extract_tagged_sequences(aligned_read, &full_ref);
            let read_tags = tags_to_output(&ets);
            if *only_output_captured_ref {
                (ets.get(&REFERENCE_CHAR).unwrap().clone(), ets.get(&READ_CHAR).unwrap().clone(), read_tags)
            } else {
                (String::from_utf8(aligned_ref.clone()).unwrap(), String::from_utf8(aligned_read.clone()).unwrap(), read_tags)
            }
        } else {
            (String::from_utf8(aligned_ref.clone()).unwrap(), String::from_utf8(aligned_read.clone()).unwrap(), String::from(""))
        };

    if *to_fake_fastq {
        let replaced = read_seq.replace("-", "");

        if replaced.len() >= *min_read_length {
            let fake_qual = (0..replaced.len()).map(|_| "H").collect::<String>();
            write!(output, "@{}_{}_ref_{}\n{}\n+\n{}\n",
                   str::replace(aligned_name, " ", "."),
                   read_tags,
                   String::from_utf8(reference_name.clone()).unwrap(),
                   replaced,
                   fake_qual,
            ).expect("Unable to write to output file");
        } else {
            warn!("Final read product too short after trimming: {}, dropping read", replaced.len());
        }
    } else {
        if read_seq.len() >= *min_read_length {
            write!(output, ">{}\n{}\n>{}_{}\n{}\n",
                   String::from_utf8(reference_name.clone()).unwrap(),
                   ref_seq,
                   str::replace(aligned_name, " ", "_"),
                   read_tags,
                   read_seq,
            ).expect("Unable to write to output file");
        } else {
            warn!("Final read product too short after trimming: {}, dropping read", read_seq.len());
        }
    }
}

pub fn simplify_cigar_string(cigar_tokens: &Vec<AlignmentTag>) -> Vec<AlignmentTag> {
    let mut new_cigar = Vec::new();

    let mut last_token: Option<AlignmentTag> = None; // zero length, so combining won't affect the final cigar string

    for token in cigar_tokens.into_iter() {
        last_token = match token {
            AlignmentTag::MatchMismatch(size) => {
                match last_token {
                    Some(AlignmentTag::MatchMismatch(size_old)) => Some(AlignmentTag::MatchMismatch(size + size_old)),
                    _ => {
                        if last_token != None { new_cigar.push(last_token.unwrap()); }

                        Some(AlignmentTag::MatchMismatch(*size))
                    }
                }
            }
            AlignmentTag::Del(size) => {
                match last_token {
                    Some(AlignmentTag::Del(size_old)) => Some(AlignmentTag::Del(size + size_old)),
                    _ => {
                        if last_token != None { new_cigar.push(last_token.unwrap()); }
                        Some(AlignmentTag::Del(*size))
                    }
                }
            }
            AlignmentTag::Ins(size) => {
                match last_token {
                    Some(AlignmentTag::Ins(size_old)) => Some(AlignmentTag::Ins(size + size_old)),
                    _ => {
                        if last_token != None { new_cigar.push(last_token.unwrap()); }
                        Some(AlignmentTag::Ins(*size))
                    }
                }
            }
            AlignmentTag::InversionOpen => {
                match last_token {
                    _ => {
                        // we don't combine inversions
                        if last_token != None { new_cigar.push(last_token.unwrap()); }
                        Some(AlignmentTag::InversionOpen)
                    }
                }
            }
            AlignmentTag::InversionClose => {
                match last_token {
                    _ => {
                        // we don't combine inversions
                        if last_token != None { new_cigar.push(last_token.unwrap()); }
                        Some(AlignmentTag::InversionClose)
                    }
                }
            }
        }
    }
    if let Some(x) = last_token {
        new_cigar.push(x);
    }
    new_cigar.reverse();
    new_cigar
}


#[cfg(test)]
mod tests {
    use libwfa::{affine_wavefront::*, bindings::*, mm_allocator::*, penalties::*};
    use rust_htslib::bam;
    use crate::alignment::alignment_matrix::{AlignmentResult, AlignmentTag};
    use crate::alignment::fasta_bit_encoding::FastaBase;
    use crate::alignment_functions::{perform_wavefront_alignment, simplify_cigar_string};

    #[test]
    fn simple_libwfa() {
        let alloc = MMAllocator::new(BUFFER_SIZE_8M as u64);

        let pattern = String::from("AAACGCTTCTGCACTTCGCGTGATATCATTACGTTTTGTCCCTCTACGATGTACCTGTCATCTTAGCTAAGACGACAGGACATGTCCAGGAAGTGCTCGAGTACTTCCTGGCCCATGTACTCTGCGTTGATACCACTGCTT");
        let text = String::from("NNNNNNNNNNNNNNNNNNNNNNNNNNNNTTACGTTNNNNNNNNNNNNGATGTACCTGTCATCTTAGCTAAGATGACAGGACATGTCCAGGAAGTACTCGAGTACTTCCTGGCCCATGTACTCTGCGTTGATACCACTGCTT");

        let mut penalties = AffinePenalties {
            match_: 0,
            mismatch: 4,
            gap_opening: 6,
            gap_extension: 2,
        };

        let pat_len = pattern.as_bytes().len();
        let text_len = text.as_bytes().len();


        for i in 0..1000 {
            let mut wavefronts = AffineWavefronts::new_complete(
                pat_len,
                text_len,
                &mut penalties,
                &alloc,
            );

            wavefronts
                .align(pattern.as_bytes(), text.as_bytes())
                .unwrap();

            let score = wavefronts.edit_cigar_score(&mut penalties);

            if i == 0 {
                println!("score: {}", score);
                wavefronts.print_cigar(pattern.as_bytes(), text.as_bytes());

                // The cigar can also be extracted as a byte vector
                let cigar = wavefronts.cigar_bytes_raw();
                let cg_str = std::str::from_utf8(&cigar).unwrap();
                println!("cigar: {}", cg_str);

                // Or as a prettier byte vector

                let cigar = wavefronts.cigar_bytes();
                let cg_str = std::str::from_utf8(&cigar).unwrap();
                println!("cigar: {}", cg_str);
            }
        }
    }

    #[test]
    fn wavefront() {
        let pattern = FastaBase::from_string(&String::from("AAACGCTTCTGCACTTCGCGTGATATCATTACGTTTTGTCCCTCTACGATGTACCTGTCATCTTAGCTAAGACGACAGGACATGTCCAGGAAGTGCTCGAGTACTTCCTGGCCCATGTACTCTGCGTTGATACCACTGCTT"));
        let text = FastaBase::from_string(&String::from("NNNNNNNNNNNNNNNNNNNNNNNNNNNNTTACGTTNNNNNNNNNNNNGATGTACCTGTCATCTTAGCTAAGATGACAGGACATGTCCAGGAAGTACTCGAGTACTTCCTGGCCCATGTACTCTGCGTTGATACCACTGCTT"));

        let alignment = perform_wavefront_alignment(&text, &pattern);

        assert_eq!(alignment.read_aligned, pattern);
        assert_eq!(alignment.reference_aligned, text);

        let pattern = FastaBase::from_string(&String::from("AAACGCTTCTGCACGTGATATCATT")); // AAACGCTTCTGCACTTCGCGTGATATCATT
        let aligned_pattern = FastaBase::from_string(&String::from("AAACGCTTCTGCAC-----GTGATATCATT"));
        let text = FastaBase::from_string(&String::from("AAACGCTTCTGCACTTCGCGTGATATCATT"));         // AAACGCTTCTGCAC-----GTGATATCATT

        let alignment = perform_wavefront_alignment(&text, &pattern);

        assert_eq!(alignment.read_aligned, aligned_pattern);
        assert_eq!(alignment.reference_aligned, text);
        assert_eq!("11M5D14M", simplify_cigar_string(&alignment.cigar_string).iter().map(|x| format!("{}", x)).collect::<Vec<String>>().join(""));

        // invert the alignment
        let alignment = perform_wavefront_alignment(&pattern, &text);

        assert_eq!(alignment.read_aligned, text);
        assert_eq!(alignment.reference_aligned, aligned_pattern);
        assert_eq!("11M5I14M", simplify_cigar_string(&alignment.cigar_string).iter().map(|x| format!("{}", x)).collect::<Vec<String>>().join(""));
    }

    #[test]
    fn writing_sam_file() {
        let mut header_record = bam::header::HeaderRecord::new(b"SQ");
        header_record.push_tag(b"SN", &"chr1");
        header_record.push_tag(b"LN", &"150");
        let mut header = bam::Header::new();
        header.push_record(&header_record);

        let mut out = bam::Writer::from_path(&"test_data/out.bam", &header, bam::Format::Bam).unwrap();
        let alignment_record = AlignmentResult {
            reference_aligned: FastaBase::from_string(&"CCAATCTACTACTGCTTGCA".to_string()),
            read_aligned: FastaBase::from_string(&"CCAATCTACTACTGCTTGCA".to_string()),
            cigar_string: vec![AlignmentTag::MatchMismatch(20)],
            path: vec![],
            score: 0.0,
            bounding_box: None,
        };

        for _i in 0..1000 {
            out.write(&alignment_record.to_sam_record("test",
                                                      &"CCAATCTACTACTGCTTGCA".to_string().into_bytes(),
                                                      &false)).unwrap();
        }
    }
}