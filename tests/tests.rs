use columnation::*;

#[test] fn test_opt_vec() { _test_pass(vec![Some(vec![0,1,2]), None]); }
#[test] fn test_option_vec() { _test_pass(vec![Some(vec![0, 1, 2])]); }
#[test] fn test_u32x3_pass() { _test_pass(vec![((1,2,3),vec![(0u32, 0u32, 0u32); 1024])]); }
#[test] fn test_u64_pass() { _test_pass(vec![0u64; 1024]); }
#[test] fn test_string_pass() { _test_pass(vec![format!("grawwwwrr!"); 1024]); }
#[test] fn test_vec_u_s_pass() { _test_pass(vec![vec![(0u64, format!("grawwwwrr!")); 32]; 32]); }

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

#[test]
fn copy_into() {
    let o = Some("test");
    let mut ts: ColumnStack<Option<String>> = ColumnStack::default();
    ts.copy_onto(&o);

    let o = Some(1);
    let mut ts: ColumnStack<Option<usize>> = ColumnStack::default();
    ts.copy_onto(&o);

    let o = ("abc", "def");
    let mut ts: ColumnStack<(String, String)> = ColumnStack::default();
    ts.copy_onto(&o);

    let o = ("abc", &o);
    let mut ts: ColumnStack<(String, (String, String))> = ColumnStack::default();
    ts.copy_onto(&o);

    let o: Result<_, ()> = Ok("asdf");
    let mut ts: ColumnStack<Result<String, ()>> = ColumnStack::default();
    ts.copy_onto(&o);

    let o = vec![("asdf", "def".to_string())];
    let mut ts: ColumnStack<Vec<(String, String)>> = ColumnStack::default();
    ts.copy_onto(&o);

    let binding = ("asdf", Some("def".to_string()));
    let o = vec![&binding];
    let mut ts: ColumnStack<Vec<(String, Option<String>)>> = ColumnStack::default();
    ts.copy_onto(&o);
    let o = [&binding];
    let mut ts: ColumnStack<Vec<(String, Option<String>)>> = ColumnStack::default();
    ts.copy_onto(&o);
}
