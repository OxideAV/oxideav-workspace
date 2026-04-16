//! Probe a .ogg's setup packet: print a summary of codebooks, floors,
//! residues, mappings, modes — so we understand what's available when
//! designing the setup-driven encoder.

fn collect_packets(data: &[u8]) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    let mut buf: Vec<u8> = Vec::new();
    let mut i = 0usize;
    while i + 27 <= data.len() {
        if &data[i..i + 4] != b"OggS" {
            break;
        }
        let n_segs = data[i + 26] as usize;
        let lacing = &data[i + 27..i + 27 + n_segs];
        let mut off = i + 27 + n_segs;
        for &lv in lacing {
            buf.extend_from_slice(&data[off..off + lv as usize]);
            off += lv as usize;
            if lv < 255 {
                out.push(std::mem::take(&mut buf));
            }
        }
        i = off;
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

fn main() {
    let path = std::env::args().nth(1).expect("pass ogg path");
    let channels: u8 = std::env::args()
        .nth(2)
        .map(|s| s.parse().unwrap_or(1))
        .unwrap_or(1);
    let data = std::fs::read(&path).unwrap();
    let pkts = collect_packets(&data);
    let setup = oxideav_vorbis::setup::parse_setup(&pkts[2], channels).expect("parse");
    println!("# codebooks: {}", setup.codebooks.len());
    for (i, cb) in setup.codebooks.iter().enumerate() {
        let vq = cb
            .vq
            .as_ref()
            .map(|v| {
                format!(
                    "vq(type={} min={} delta={} vbits={} seq={})",
                    v.lookup_type, v.min, v.delta, v.value_bits, v.sequence_p
                )
            })
            .unwrap_or_else(|| "no_vq".to_string());
        let used = cb.codeword_lengths.iter().filter(|&&l| l > 0).count();
        let max_len = cb.codeword_lengths.iter().copied().max().unwrap_or(0);
        println!(
            "  cb{:>3}: dim={} entries={} used={} max_len={} {}",
            i, cb.dimensions, cb.entries, used, max_len, vq
        );
    }
    println!("# floors: {}", setup.floors.len());
    for (i, f) in setup.floors.iter().enumerate() {
        match f {
            oxideav_vorbis::setup::Floor::Type1(f1) => {
                println!(
                    "  floor{}: type1 partitions={} classes={} mult={} rangebits={} xlist_len={}",
                    i,
                    f1.partition_class_list.len(),
                    f1.class_dimensions.len(),
                    f1.multiplier,
                    f1.rangebits,
                    f1.xlist.len(),
                );
                println!("    partition_class_list: {:?}", f1.partition_class_list);
                println!("    class_dimensions: {:?}", f1.class_dimensions);
                println!("    class_subclasses: {:?}", f1.class_subclasses);
                println!("    class_masterbook: {:?}", f1.class_masterbook);
                for (ci, sb) in f1.class_subbook.iter().enumerate() {
                    println!("    class{}.subbook: {:?}", ci, sb);
                }
                println!("    xlist: {:?}", f1.xlist);
            }
            oxideav_vorbis::setup::Floor::Type0(_) => println!("  floor{}: type0 unsupported", i),
        }
    }
    println!("# residues: {}", setup.residues.len());
    for (i, r) in setup.residues.iter().enumerate() {
        println!(
            "  res{}: kind={} begin={} end={} psz={} classifications={} classbook={}",
            i, r.kind, r.begin, r.end, r.partition_size, r.classifications, r.classbook,
        );
        println!("    cascade: {:?}", r.cascade);
        for (c, books) in r.books.iter().enumerate() {
            print!("    class{} books:", c);
            for b in books.iter() {
                print!(" {}", b);
            }
            println!();
        }
    }
    println!("# mappings: {}", setup.mappings.len());
    for (i, m) in setup.mappings.iter().enumerate() {
        println!(
            "  map{}: submaps={} coupling={:?} mux={:?} floors={:?} residues={:?}",
            i, m.submaps, m.coupling, m.mux, m.submap_floor, m.submap_residue,
        );
    }
    println!("# modes: {}", setup.modes.len());
    for (i, md) in setup.modes.iter().enumerate() {
        println!(
            "  mode{}: blockflag={} mapping={}",
            i, md.blockflag, md.mapping
        );
    }
}
