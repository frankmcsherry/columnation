use columnation::*;

fn main() {

    // profile_copy(vec![0u64; 1024]);
    profile_copy(vec![format!("grawwwwrr!"); 1024]);

}

fn profile_copy<T: Columnation+Eq>(record: T) {

    let mut arena = ColumnStack::<T>::default();
    // prepare encoded data for bencher.bytes
    let timer = std::time::Instant::now();
    for _ in 0 .. 1000 {
        arena.clear();
        for _ in 0 .. 1024 {
            arena.copy(&record);
        }
    }
    println!("{:?} elapsed", timer.elapsed());
}
