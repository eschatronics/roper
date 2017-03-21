#[allow(dead_code)]
extern crate elf;
extern crate unicorn;
extern crate capstone;
extern crate rand;
extern crate getopts;
extern crate scoped_threadpool;

use scoped_threadpool::Pool;
use std::sync::mpsc::channel;
use getopts::Options;
use std::env;

use std::fs::{File,OpenOptions};
use std::io::prelude::*;
mod roper;

use rand::{thread_rng,Rng,Generator};

use std::path::{Path,PathBuf};
use std::sync::{Mutex,Arc,RwLock};
use std::cmp::Ordering;
use unicorn::*;
// use std::io;
//use roper::dis::{disas_sec,Inst};
use roper::thumb::*;
use roper::util::*;
use roper::population::*;
use roper::hatchery::*;
use roper::phylostructs::*;
use roper::evolution::*;
use roper::ontostructs::*;
use roper::csv_reader::*;

fn print_usage (program: &str, opts: Options) {
  let brief = format!("Usage: {} [options]", program);
  print!("{}", opts.usage(&brief));
}

fn load_file (path: &str) -> Vec<u8>
{
  let mut f = File::open(path)
                .expect("Failed to open path");
  let mut buf : Vec<u8> = Vec::new();
  f.read_to_end(&mut buf);
  buf
}

fn get_elf_addr_data (path: &str, 
                      secs: &Vec<&str>) 
                      -> Vec<Sec> {
  let path = PathBuf::from(path);
  let file = match elf::File::open_path(&path) {
    Ok(f) => f,
    Err(e) => panic!("Error: {:?}",e),
  };
  let mut sections : Vec<Sec> = Vec::new();
  for sec_name in secs.iter() {
    let sec = file.get_section(sec_name)
                  .expect("Unable to fetch section from elf");
    sections.push(Sec {
      name: sec_name.to_string(),
      addr: sec.shdr.addr,
      data: sec.data.clone(),
      perm: PROT_ALL, // Placeholder. Need to convert from elf
    });
  }
  sections
}

fn get_gba_addr_data (path: &str) -> Vec<(u64, Vec<u8>)> {
  let addr = GBA_CARTRIDGE_ROM_START;
  let data = load_file(path);
  vec![(addr,data)]
}
                    

const GBA_CARTRIDGE_ROM_START : u64 = 0x08000000;

/* Just a debugging stub */
fn main() {
  let disas_path = "/tmp/roper_disassembly.txt"; 
  let args: Vec<String> = env::args().collect();
  let program = args[0].clone();

  let mut opts = Options::new();
  let verbose = true;
  opts.optopt("p", "", "set target pattern", "PATTERN");
  opts.optopt("d", "", "set data path", "PATH");
  opts.optopt("g", "", "set fitness goal (default 0)", "POSITIVE FLOAT <= 1");
  opts.optopt("o", "", "set log directory", "DIRECTORY");
  opts.optopt("h", "help", "print this help menu", "");
  opts.optopt("t", "threads", "set number of threads", "");
  opts.optopt("P", "population", "set population size", "");
  opts.optflag("V", "viscosity", "use viscosity modulations to encourage gene linkage");
  opts.optopt("D", "demes", "set number of demes", "");
  opts.optopt("b", "binary", "select binary file to search for gadgets", "");
  let matches = match opts.parse(&args[1..]) {
    Ok(m)  => { m },
    Err(f) => { panic!(f.to_string()) },
  };
  if matches.opt_present("h") {
    print_usage(&program, opts);
    return;
  }
  let use_viscosity = true;
  /*
    if matches.opt_present("V") {
      true
    } else {
      false
    };
    */
  println!(">> use_viscosity = {}", use_viscosity);
  let popsize = match matches.opt_str("P") {
    None => 2000,
    Some(n) => n.parse::<usize>().unwrap(),
  };
  let num_demes = match matches.opt_str("D") {
    None => 4,
    Some(n) => n.parse::<usize>().unwrap(),
  };
  let rpattern_str = matches.opt_str("p");
  let data_path    = matches.opt_str("d");
  let threads : usize = match matches.opt_str("t") {
    None => 8,
    Some(n) => n.parse::<usize>().unwrap(),
  };
  let log_dir      = match matches.opt_str("o") {
    None    => {
      let p = Path::new("./logs/");
      if p.is_dir() { 
        p.to_str().unwrap().to_string() 
      } else { 
      "./".to_string()
      }
    },
    Some(p) => p,
  };
  let goal : f32 = match matches.opt_str("g") {
    None => 0.0,
    Some(s) => s.parse::<f32>()
                .expect("Error parsing fitness goal"),
  };
  println!(">> goal = {}", goal);
  // ugly kludge here
  let (io_targets, pattern_matching) : (IoTargets,bool) =
    match (rpattern_str, data_path) {
      (Some(s),None) => (IoTargets::from_vec(TargetKind::PatternMatch,
                                             vec![(vec![1;16], 
                                                  Target::Exact(
                                                  RPattern::new(&s)
                                                  ))]
                                             ),true),
      (None,Some(s)) => (process_data2(&s,4).shuffle(),false), // don't hardcode numfields. infer by analysing lines. 
      _              => {
        print_usage(&program, opts);
        return;
      },
    };
  
  let (testing,training) = io_targets.split_at(io_targets.len()/3);
  let mut debug_samples = training.clone();
  /**************************************************/
  let sample1 = "tomato-RT-AC3200-ARM-132-AIO-httpd";
  let sample2 = "tomato-RT-N18U-httpd";
  let sample3 = "openssl";
  let sample4 = "ldconfig.real";
  let sample_gba = "megaman_zero_4.gba";
  let sample_root = "/home/oblivia/Projects/roper/data/"
    .to_string();
  let elf_path = sample_root.clone() + sample4;
  let gba_path = sample_root.clone() + sample_gba;
  let elf_addr_data = get_elf_addr_data(&elf_path,
                                        &vec![".text",".rodata"]);
  println!("****************** ELF {} **********************",
           elf_path);
  let text_addr = elf_addr_data[0].addr;
  let text_data = &elf_addr_data[0].data;
  let rodata_addr = elf_addr_data[1].addr;
  let rodata_data = &elf_addr_data[1].data;
  let wordvec_elf = u8s_to_u16s(&text_data, Endian::LITTLE);
  
  let mode = MachineMode::ARM;

  let iris_data = sample_root.clone() + "/iris.data";

  let elf_clumps = reap_gadgets(text_data,
                                text_addr as u32,
                                mode);

  let constants = suggest_constants(&io_targets);
  let mut params : Params = Params::new();
  let num_targets = io_targets.len();
  if pattern_matching {
    params.outregs = vec![0,1,2,3,4,5,6,7,8,9,10,11,12,13,14];
  }
  params.code = text_data.clone();
  params.code_addr = text_addr as u32;
  params.data = vec![rodata_data.clone()];
  params.data_addrs   = vec![rodata_addr as u32];
  params.constants    = constants;
  params.io_targets   = training;
  params.test_targets = testing;
  params.fit_goal     = goal;
  params.verbose      = verbose;
  params.threads      = threads;
  params.num_demes    = num_demes;
  params.use_viscosity = use_viscosity;
  params.set_log_dir(&log_dir);
  params.population_size = popsize;
  params.binary_path = elf_path.clone();
  // add string search function
  // find string addresses in rodata
  // pass these addresses to the mangler in population building
  //println!("params: {:?}",params); 

  let mut rng = rand::thread_rng();
  let mut population = Population::new(&params);

  let mut machinery : Machinery
    = Machinery::new(&elf_path,
                     mode,
                     threads,
                     false);

  let mut debug_machinery : Machinery 
    = Machinery::new(&elf_path,
                     mode,
                     1,
                     true);
  add_debug_hooks(debug_machinery.cluster[0].unwrap_mut());
  let mut champion : Option<Chain> = None;
  let pop_rw  = RwLock::new(population);
  let pop_arc = Arc::new(pop_rw); 
  let pop_local = pop_arc.clone();
  let mut i = 0; 
  while pop_local.read().unwrap().generation < pop_local.read().unwrap().params.max_generations &&
    (pop_local.read().unwrap().best_fit() == None 
    || pop_local.read().unwrap().best_crashes() == Some(true)
    || pop_local.read().unwrap().best_fit() > Some(params.fit_goal)){
    
    let (tx, rx)  = channel();
    let n_workers = threads as u32;
    let n_jobs    = machinery.cluster.len();
    let mut pool  = Pool::new(n_workers);
    pool.scoped(|scope| {
      let mut vdeme = 0;
      for e in machinery.cluster.iter_mut() {
        let tx = tx.clone();
        let p = pop_arc.clone();
        scope.execute(move || {
          let t = tournement(&p.read().unwrap(),
                             e,
                             Batch::TRAINING,
                             vdeme);
          tx.send(t).unwrap();
        });
        vdeme = (vdeme + 1) % num_demes;
      }
      let mut trs : Vec<TournementResult> = rx.iter()
                                              .take(n_jobs)
                                              .collect();
      println!("");
      trs.sort_by(|a,b| b.best.fitness
                         .partial_cmp(&a.best.fitness)
                         .unwrap_or(Ordering::Equal));
      for tr in trs {
        //println!("{:?}",tr);
        let updated = patch_population(tr, &mut pop_local.write().unwrap());
        if updated != None {
          champion = updated.clone();
        }
        if false && updated != None  { // DISABLED FOR NOW
          println!("[*] Running best with disassembly on...");
          debug_samples.shuffle();
          let targets = debug_samples.split_at(2).0;
          evaluate_fitness(debug_machinery.cluster[0].unwrap_mut(),
                           &mut updated.unwrap(),
                           &pop_local.read().unwrap().params,
                           true);
          
          let mut dfile = OpenOptions::new()
                                      .append(true)
                                      .write(true)
                                      .create(true)
                                      .open(&disas_path)
                                      .unwrap();
          dfile.write("==== END OF EVALUATION ====\n".as_bytes()).unwrap();
          println!("[*] Finished running best with dissassembly");
        }
      }
  
      pop_local.read().unwrap().periodic_save();

      let avg_pop_gen = pop_local.read()
                                 .unwrap()
                                 .avg_gen();
      let avg_pop_fit = pop_local.read()
                                 .unwrap()
                                 .avg_fit();
      println!("==> AVG POP GEN: {}", avg_pop_gen);
      println!("==> AVG POP FIT: {}", avg_pop_fit);
    }); // END POOL SCOPE
    i += 1;
  }
  println!("=> {} GENERATIONS", pop_local.read().unwrap().generation);
  println!("=> BEST FIT: {:?}", pop_local.read().unwrap().best_fit());
  println!("=> RUNNING BEST:\n");
  let targets = pop_local.read().unwrap().params.test_targets.clone();
  println!("ok, got targets...");
  if champion == None {
    panic!("Champion is none!");
  }
  evaluate_fitness(debug_machinery.cluster[0].unwrap_mut(),
                   &mut champion.unwrap(),
                   &pop_local.read().unwrap().params,
                   true);
  println!("\n{}", pop_local.read().unwrap().best.clone().unwrap());
  
}
