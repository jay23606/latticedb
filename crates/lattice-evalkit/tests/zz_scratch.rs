use lattice_evalkit::metrics::*;
use lattice_evalkit::drift::*;
fn pair(e: &[&str], g: &[&str]) -> (Vec<String>, Vec<String>) {
    (e.iter().map(|s| s.to_string()).collect(), g.iter().map(|s| s.to_string()).collect())
}
#[test]
fn print() {
    let v = [0.1, 0.4, 0.5, 0.9, 1.0, 0.3, 0.7, 0.2];
    println!("CI {:?}", bootstrap_ci(&v, 200, 42, 0.9));
    println!("CI2 {:?}", bootstrap_ci(&v, 1, 7, 0.95));
    let pairs = vec![pair(&["a","b"],&["a"]), pair(&["c"],&["c","d"]), pair(&[],&["x"]), pair(&["y"],&["y"]), pair(&["z"],&[])];
    println!("MP {:?}", bootstrap_micro_prf(&pairs, 200, 5, 0.9));
    let d = generate(11, 40, 30);
    println!("HASH {}", d.hash);
    println!("EX0 {:?}", d.examples.iter().take(3).map(|e| (e.input.clone(), e.expected.clone())).collect::<Vec<_>>());
    println!("ECE {}", ece(&[(0.9,true),(0.9,false),(0.2,false)], 10));
}
