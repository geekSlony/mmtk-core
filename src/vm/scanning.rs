use crate::plan::{Mutator, SelectedPlan, TraceLocal, TransitiveClosure};
use crate::scheduler::gc_works::ProcessEdgesWork;
use crate::util::ObjectReference;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;

pub trait Scanning<VM: VMBinding> {
    /// Scan stack roots after all mutators are paused
    const SCAN_MUTATORS_IN_SAFEPOINT: bool = true;
    /// Scan all the mutators within a single work packet
    ///
    /// `SCAN_MUTATORS_IN_SAFEPOINT` should also be enabled
    const SINGLE_THREAD_MUTATOR_SCANNING: bool = true;
    fn scan_object<T: TransitiveClosure>(
        trace: &mut T,
        object: ObjectReference,
        tls: OpaquePointer,
    );
    fn reset_thread_counter();
    fn notify_initial_thread_scan_complete(partial_scan: bool, tls: OpaquePointer);
    /// Scan all thread roots and create `RootsEdge` work packets
    ///
    /// TODO: Smaller work granularity
    fn scan_objects<W: ProcessEdgesWork<VM = VM>>(objects: &[ObjectReference]);
    /// Scan all the mutators for roots
    fn scan_thread_roots<W: ProcessEdgesWork<VM = VM>>();
    /// Scan one mutator for roots
    fn scan_thread_root<W: ProcessEdgesWork<VM = VM>>(
        mutator: &'static mut Mutator<SelectedPlan<VM>>,
    );
    /// The creation of all root scan tasks (except thread scanning) goes here
    fn scan_vm_specific_roots<W: ProcessEdgesWork<VM = VM>>();
    fn compute_static_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer);
    fn compute_global_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer);
    fn compute_thread_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer);
    fn compute_new_thread_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer);
    fn compute_bootimage_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer);
    fn supports_return_barrier() -> bool;
}
