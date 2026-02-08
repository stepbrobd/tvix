use criterion::{Criterion, black_box, criterion_group, criterion_main};
use mimalloc::MiMalloc;
use std::{env, rc::Rc, time::Duration};
use tvix_eval::{EvalIO, builtins::impure_builtins};
use tvix_glue::{
    builtins::{add_derivation_builtins, add_import_builtins},
    configure_nix_path,
    tvix_io::TvixIO,
    tvix_store_io::TvixStoreIO,
};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn interpret(code: &str) {
    // We assemble a complete store in memory.
    let tvix_store_io = Rc::new(TvixStoreIO::new(Default::default()));

    let mut eval_builder = tvix_eval::Evaluation::builder(Rc::new(TvixIO::new(
        tvix_store_io.clone() as Rc<dyn EvalIO>,
    )) as Rc<dyn EvalIO>)
    .enable_import()
    .add_builtins(impure_builtins());

    eval_builder = add_derivation_builtins(eval_builder, Rc::clone(&tvix_store_io));
    // eval_builder = add_fetcher_builtins(eval_builder, Rc::clone(&tvix_store_io));
    eval_builder = add_import_builtins(eval_builder, tvix_store_io);
    eval_builder = configure_nix_path(
        eval_builder,
        // The benchmark requires TVIX_BENCH_NIX_PATH to be set, so barf out
        // early, rather than benchmarking tvix returning an error.
        &Some(env::var("TVIX_BENCH_NIX_PATH").expect("TVIX_BENCH_NIX_PATH must be set")),
    );

    let eval = eval_builder.build();
    let result = eval.evaluate(code, None);

    assert!(result.errors.is_empty(), "{:#?}", result.errors);
}

fn eval_nixpkgs(c: &mut Criterion) {
    c.bench_function("hello outpath", |b| {
        b.iter(|| {
            interpret(black_box("(import <nixpkgs> {}).hello.outPath"));
        })
    });

    c.bench_function("firefox outpath", |b| {
        b.iter(|| {
            interpret(black_box("(import <nixpkgs> {}).firefox.outPath"));
        })
    });
}

criterion_group!(
    name = benches;
    config = Criterion::default().measurement_time(Duration::from_secs(30)).sample_size(10);
    targets = eval_nixpkgs
);
criterion_main!(benches);
