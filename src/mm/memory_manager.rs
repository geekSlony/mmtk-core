//! VM-to-MMTk interface: safe Rust APIs.
//!
//! This module provides a safe Rust API for mmtk-core.
//! We expect the VM binding to inherit and extend this API by:
//! 1. adding their VM-specific functions
//! 2. exposing the functions to native if necessary. And the VM binding needs to manage the unsafety
//!    for exposing this safe API to FFI.
//!
//! For example, for mutators, this API provides a `Box<Mutator>`, and requires a `&mut Mutator` for allocation.
//! A VM binding can borrow a mutable reference directly from `Box<Mutator>`, and call `alloc()`. Alternatively,
//! it can turn the `Box` pointer to a native pointer (`*mut Mutator`), and forge a mut reference from the native
//! pointer. Either way, the VM binding code needs to guarantee the safety.

use std::sync::atomic::Ordering;

use crate::plan::mutator_context::{Mutator, MutatorContext};
use crate::plan::Plan;
use crate::scheduler::GCWorker;

use crate::vm::Collection;

use crate::util::{Address, ObjectReference};

use self::selected_plan::SelectedPlan;
use crate::plan::selected_plan;
use crate::util::alloc::allocators::AllocatorSelector;

use crate::mmtk::MMTK;
use crate::plan::AllocationSemantics;
use crate::util::constants::LOG_BYTES_IN_PAGE;
use crate::util::heap::layout::vm_layout_constants::HEAP_END;
use crate::util::heap::layout::vm_layout_constants::HEAP_START;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;

/// Run the main loop for the GC controller thread. This method does not return.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `tls`: The thread that will be used as the GC controller.
pub fn start_control_collector<VM: VMBinding>(mmtk: &MMTK<VM>, tls: OpaquePointer) {
    mmtk.plan.base().control_collector_context.run(tls);
}

/// Initialize an MMTk instance. A VM should call this method after creating an [MMTK](../mmtk/struct.MMTK.html)
/// instance but before using any of the methods provided in MMTk.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance to initialize.
/// * `heap_size`: The heap size for the MMTk instance in bytes.
pub fn gc_init<VM: VMBinding>(mmtk: &'static mut MMTK<VM>, heap_size: usize) {
    crate::util::logger::init().unwrap();
    mmtk.plan.gc_init(heap_size, &mmtk.vm_map, &mmtk.scheduler);
}

/// Request MMTk to create a mutator for the given thread. For performance reasons, A VM should
/// store the returned mutator in a thread local storage that can be accessed efficiently.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `tls`: The thread that will be associated with the mutator.
pub fn bind_mutator<VM: VMBinding>(
    mmtk: &'static MMTK<VM>,
    tls: OpaquePointer,
) -> Box<Mutator<SelectedPlan<VM>>> {
    SelectedPlan::bind_mutator(&mmtk.plan, tls, mmtk)
}

/// Reclaim a mutator that is no longer needed.
///
/// Arguments:
/// * `mutator`: A reference to the mutator to be destroyed.
pub fn destroy_mutator<VM: VMBinding>(mutator: Box<Mutator<SelectedPlan<VM>>>) {
    drop(mutator);
}

/// Flush the mutator's local states.
///
/// Arguments:
/// * `mutator`: A reference to the mutator.
pub fn flush_mutator<VM: VMBinding>(mutator: &mut Mutator<SelectedPlan<VM>>) {
    mutator.flush()
}

/// Allocate memory for an object. For performance reasons, a VM should
/// implement the allocation fast-path on their side rather than just calling this function.
///
/// Arguments:
/// * `mutator`: The mutator to perform this allocation request.
/// * `size`: The number of bytes required for the object.
/// * `align`: Required alignment for the object.
/// * `offset`: Offset associated with the alignment.
/// * `semantics`: The allocation semantic required for the allocation.
pub fn alloc<VM: VMBinding>(
    mutator: &mut Mutator<SelectedPlan<VM>>,
    size: usize,
    align: usize,
    offset: isize,
    semantics: AllocationSemantics,
) -> Address {
    mutator.alloc(size, align, offset, semantics)
}

/// Perform post-allocation actions, usually initializing object metadata. For many allocators none are
/// required. For performance reasons, a VM should implement the post alloc fast-path on their side
/// rather than just calling this function.
///
/// Arguments:
/// * `mutator`: The mutator to perform post-alloc actions.
/// * `refer`: The newly allocated object.
/// * `type_refer`: The type reference for the instance being created (unused).
/// * `bytes`: The size of the space allocated for the object (in bytes).
/// * `semantics`: The allocation semantics used for the allocation.
pub fn post_alloc<VM: VMBinding>(
    mutator: &mut Mutator<SelectedPlan<VM>>,
    refer: ObjectReference,
    type_refer: ObjectReference,
    bytes: usize,
    semantics: AllocationSemantics,
) {
    mutator.post_alloc(refer, type_refer, bytes, semantics);
}

/// Return an AllocatorSelector for the given allocation semantic. This method is provided
/// so that VM compilers may call it to help generate allocation fast-path.
///
/// Arguments:
/// * `mmtk`: The reference to an MMTk instance.
/// * `semantics`: The allocation semantic to query.
pub fn get_allocator_mapping<VM: VMBinding>(
    mmtk: &MMTK<VM>,
    semantics: AllocationSemantics,
) -> AllocatorSelector {
    mmtk.plan.get_allocator_mapping()[semantics]
}

/// Run the main loop of a GC worker. This method does not return.
///
/// Arguments:
/// * `tls`: The thread that will be used as the GC worker.
/// * `worker`: A reference to the GC worker.
/// * `mmtk`: A reference to an MMTk instance.
pub fn start_worker<VM: VMBinding>(
    tls: OpaquePointer,
    worker: &'static mut GCWorker<VM>,
    mmtk: &'static MMTK<VM>,
) {
    worker.init(tls);
    worker.run(mmtk);
}

/// Allow MMTk to trigger garbage collection. A VM should only call this method when it is ready for the mechanisms required for
/// collection during the boot process.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `tls`: The thread that wants to enable the collection.
pub fn enable_collection<VM: VMBinding>(mmtk: &'static MMTK<VM>, tls: OpaquePointer) {
    mmtk.scheduler.initialize(mmtk.options.threads, mmtk, tls);
    VM::VMCollection::spawn_worker_thread(tls, None); // spawn controller thread
    mmtk.plan.base().initialized.store(true, Ordering::SeqCst);
}

/// Process MMTk run-time options.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `name`: The name of the option.
/// * `value`: The value of the option (as a string).
pub fn process<VM: VMBinding>(mmtk: &'static MMTK<VM>, name: &str, value: &str) -> bool {
    unsafe { mmtk.options.process(name, value) }
}

/// Return used memory in bytes.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
pub fn used_bytes<VM: VMBinding>(mmtk: &MMTK<VM>) -> usize {
    mmtk.plan.get_pages_used() << LOG_BYTES_IN_PAGE
}

/// Return free memory in bytes.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
pub fn free_bytes<VM: VMBinding>(mmtk: &MMTK<VM>) -> usize {
    mmtk.plan.get_free_pages() << LOG_BYTES_IN_PAGE
}

/// Return the starting address of the heap. *Note that currently MMTk uses
/// a fixed address range as heap.*
pub fn starting_heap_address() -> Address {
    HEAP_START
}

/// Return the ending address of the heap. *Note that currently MMTk uses
/// a fixed address range as heap.*
pub fn last_heap_address() -> Address {
    HEAP_END
}

/// Return the total memory in bytes.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
pub fn total_bytes<VM: VMBinding>(mmtk: &MMTK<VM>) -> usize {
    mmtk.plan.get_total_pages() << LOG_BYTES_IN_PAGE
}

/// Perform a linear scan through a single contiguous region.
#[cfg(feature = "sanity")]
#[deprecated]
pub fn scan_region() {
    crate::util::sanity::memory_scan::scan_region();
}

/// Trigger a garbage collection as requested by the user.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `tls`: The thread that triggers this collection request.
pub fn handle_user_collection_request<VM: VMBinding>(mmtk: &MMTK<VM>, tls: OpaquePointer) {
    mmtk.plan.handle_user_collection_request(tls, false);
}

/// Is the object alive?
///
/// Arguments:
/// * `object`: The object reference to query.
pub fn is_live_object(object: ObjectReference) -> bool {
    object.is_live()
}

/// Is the object in the mapped memory?
///
/// Arguments:
/// * `object`: The object reference to query.
pub fn is_mapped_object(object: ObjectReference) -> bool {
    object.is_mapped()
}

/// Is the address in the mapped memory?
///
/// Arguments:
/// * `address`: The address to query.
pub fn is_mapped_address(address: Address) -> bool {
    address.is_mapped()
}

/// Check that if a garbage collection is in progress and if the given
/// object is not movable.  If it is movable error messages are
/// logged and the system exits.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `object`: The object to check.
pub fn modify_check<VM: VMBinding>(mmtk: &MMTK<VM>, object: ObjectReference) {
    mmtk.plan.modify_check(object);
}

/// Add a reference to the list of weak references.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `reff`: The weak reference to add.
/// * `referent`: The object that the reference points to.
pub fn add_weak_candidate<VM: VMBinding>(
    mmtk: &MMTK<VM>,
    reff: ObjectReference,
    referent: ObjectReference,
) {
    mmtk.reference_processors
        .add_weak_candidate::<VM>(reff, referent);
}

/// Add a reference to the list of soft references.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `reff`: The soft reference to add.
/// * `referent`: The object that the reference points to.
pub fn add_soft_candidate<VM: VMBinding>(
    mmtk: &MMTK<VM>,
    reff: ObjectReference,
    referent: ObjectReference,
) {
    mmtk.reference_processors
        .add_soft_candidate::<VM>(reff, referent);
}

/// Add a reference to the list of phantom references.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `reff`: The phantom reference to add.
/// * `referent`: The object that the reference points to.
pub fn add_phantom_candidate<VM: VMBinding>(
    mmtk: &MMTK<VM>,
    reff: ObjectReference,
    referent: ObjectReference,
) {
    mmtk.reference_processors
        .add_phantom_candidate::<VM>(reff, referent);
}

/// Generic hook to allow benchmarks to be harnessed. We do a full heap
/// GC, and then start recording statistics for MMTk.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
/// * `tls`: The thread that calls the function (and triggers a collection).
pub fn harness_begin<VM: VMBinding>(mmtk: &MMTK<VM>, tls: OpaquePointer) {
    mmtk.harness_begin(tls);
}

/// Generic hook to allow benchmarks to be harnessed. We stop collecting
/// statistics, and print stats values.
///
/// Arguments:
/// * `mmtk`: A reference to an MMTk instance.
pub fn harness_end<VM: VMBinding>(mmtk: &'static MMTK<VM>) {
    mmtk.harness_end();
}
