//! Running per-file work on several threads, with results in input order.

use rayon::prelude::*;

/// The number of threads to use when `-j` is not given: one per CPU.
pub fn default_jobs() -> usize {
    std::thread::available_parallelism().map_or(1, usize::from)
}

/// `f` applied to every item, `jobs` at a time, with the results in the
/// order of `items`. One item, or one job, runs on the calling thread
/// without starting a pool: an editor formatting one file on save should
/// not pay for threads.
pub fn map<T, R, F>(items: Vec<T>, jobs: usize, f: F) -> Vec<R>
where
    T: Send,
    R: Send,
    F: Fn(T) -> R + Sync + Send,
{
    if jobs <= 1 || items.len() <= 1 {
        return items.into_iter().map(f).collect();
    }
    match rayon::ThreadPoolBuilder::new()
        .num_threads(jobs.min(items.len()))
        .build()
    {
        Ok(pool) => pool.install(|| items.into_par_iter().map(f).collect()),
        // Without threads the work still gets done, on this one.
        Err(_) => items.into_iter().map(f).collect(),
    }
}
