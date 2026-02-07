use columnation::*;

#[test] fn test_opt_vec() { _test_pass(vec![Some(vec![0,1,2]), None]); }
#[test] fn test_option_vec() { _test_pass(vec![Some(vec![0, 1, 2])]); }
#[test] fn test_u32x3_pass() { _test_pass(vec![((1,2,3),vec![(0u32, 0u32, 0u32); 1024])]); }
#[test] fn test_u64_pass() { _test_pass(vec![0u64; 1024]); }
#[test] fn test_string_pass() { _test_pass(vec![format!("grawwwwrr!"); 1024]); }
#[test] fn test_vec_u_s_pass() { _test_pass(vec![vec![(0u64, format!("grawwwwrr!")); 32]; 32]); }
#[test]
fn test_smallvec() {
    use smallvec::SmallVec;
    let mut v: SmallVec<[i32; 1]> = SmallVec::with_capacity(2);
    assert!(v.spilled());
    v.push(42);
    _test_pass(v);
}

fn _test_pass<T: Columnation+Eq>(record: T) {

    // prepare encoded data for bencher.bytes
    let mut arena = ColumnStack::<T>::default();
    for _ in 0 .. 100 {
        arena.copy(&record);
    }
    for element in arena.iter() {
        assert!(element == &record);
    }
}
