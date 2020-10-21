use super::global::GenCopy;
use crate::scheduler::gc_works::*;
use crate::scheduler::{GCWorker, GCWork};
use crate::util::{Address, ObjectReference, OpaquePointer};
use crate::vm::*;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use crate::policy::space::Space;
use crate::util::alloc::{BumpAllocator, Allocator};
use crate::util::forwarding_word;
use crate::MMTK;
use crate::plan::{CopyContext, Plan};

pub struct GenCopyCopyContext<VM: VMBinding> {
    plan: &'static GenCopy<VM>,
    ss: BumpAllocator<VM>,
}

impl <VM: VMBinding> CopyContext for GenCopyCopyContext<VM> {
    type VM = VM;
    fn new(mmtk: &'static MMTK<Self::VM>) -> Self {
        Self {
            plan: unsafe { &*(&mmtk.plan as *const _ as *const GenCopy<VM>) },
            ss: BumpAllocator::new(OpaquePointer::UNINITIALIZED, None, &mmtk.plan),
        }
    }
    fn prepare(&mut self) {
        self.ss.rebind(Some(self.plan.tospace()));
    }
    fn release(&mut self) {
        // self.ss.rebind(Some(self.plan.tospace()));
    }
    #[inline(always)]
    fn alloc_copy(&mut self, _original: ObjectReference, bytes: usize, align: usize, offset: isize, _allocator: crate::Allocator) -> Address {
        debug_assert!(VM::VMActivePlan::global().base().gc_in_progress_proper());
        self.ss.alloc(bytes, align, offset)
    }
    #[inline(always)]
    fn post_copy(&mut self, obj: ObjectReference, _tib: Address, _bytes: usize, _allocator: crate::Allocator) {
        forwarding_word::clear_forwarding_bits::<VM>(obj);
    }
}




#[derive(Default)]
pub struct GenCopyNurseryProcessEdges<VM: VMBinding>  {
    base: ProcessEdgesBase<GenCopyNurseryProcessEdges<VM>>,
    phantom: PhantomData<VM>,
}

impl <VM: VMBinding> ProcessEdgesWork for GenCopyNurseryProcessEdges<VM> {
    type VM = VM;
    fn new(edges: Vec<Address>, _roots: bool) -> Self {
        Self { base: ProcessEdgesBase::new(edges), ..Default::default() }
    }
    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        // Evacuate nursery objects
        if self.plan().nursery.in_space(object) {
            return self.plan().nursery.trace_object(self, object, super::global::ALLOC_SS, self.worker().local());
        }
        debug_assert!(!self.plan().fromspace().in_space(object));
        debug_assert!(self.plan().tospace().in_space(object));
        return object;
    }
    #[inline]
    fn process_edge(&mut self, slot: Address) {
        debug_assert!(!self.plan().fromspace().address_in_space(slot));
        let object = unsafe { slot.load::<ObjectReference>() };
        let new_object = self.trace_object(object);
        debug_assert!(!self.plan().nursery.in_space(new_object));
        if Self::OVERWRITE_REFERENCE {
            unsafe { slot.store(new_object) };
        }
    }
}

impl <VM: VMBinding> Deref for GenCopyNurseryProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl <VM: VMBinding> DerefMut for GenCopyNurseryProcessEdges<VM> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}



#[derive(Default)]
pub struct GenCopyMatureProcessEdges<VM: VMBinding>  {
    base: ProcessEdgesBase<GenCopyMatureProcessEdges<VM>>,
    phantom: PhantomData<VM>,
}

impl <VM: VMBinding> ProcessEdgesWork for GenCopyMatureProcessEdges<VM> {
    type VM = VM;
    fn new(edges: Vec<Address>, _roots: bool) -> Self {
        Self { base: ProcessEdgesBase::new(edges), ..Default::default() }
    }
    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        // Evacuate nursery objects
        if self.plan().nursery.in_space(object) {
            return self.plan().nursery.trace_object(self, object, super::global::ALLOC_SS, self.worker().local());
        }
        // Evacuate mature objects
        if self.plan().tospace().in_space(object) {
            return self.plan().tospace().trace_object(self, object, super::global::ALLOC_SS, self.worker().local());
        }
        if self.plan().fromspace().in_space(object) {
            return self.plan().fromspace().trace_object(self, object, super::global::ALLOC_SS, self.worker().local());
        }
        self.plan().common.trace_object(self, object)
    }
}

impl <VM: VMBinding> Deref for GenCopyMatureProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl <VM: VMBinding> DerefMut for GenCopyMatureProcessEdges<VM> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

#[derive(Default)]
pub struct GenCopyProcessModBuf {
    pub modified_nodes: Vec<ObjectReference>,
    pub modified_edges: Vec<Address>,
}

impl <VM: VMBinding> GCWork<VM> for GenCopyProcessModBuf {
    #[inline]
    fn do_work(&mut self, worker: &'static mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        if mmtk.plan.in_nursery() {
            let mut modified_nodes = vec![];
            ::std::mem::swap(&mut modified_nodes, &mut self.modified_nodes);
            worker.scheduler().closure_stage.add(ScanObjects::<GenCopyNurseryProcessEdges::<VM>>::new(modified_nodes, false));

            let mut modified_edges = vec![];
            ::std::mem::swap(&mut modified_edges, &mut self.modified_edges);
            worker.scheduler().closure_stage.add(GenCopyNurseryProcessEdges::<VM>::new(modified_edges, true));
        } else {
            // Do nothing
        }
    }
}



#[derive(Default)]
pub struct SanityGCProcessEdges<VM: VMBinding>  {
    base: ProcessEdgesBase<SanityGCProcessEdges<VM>>,
    phantom: PhantomData<VM>,
}

impl <VM: VMBinding> Deref for SanityGCProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl <VM: VMBinding> DerefMut for SanityGCProcessEdges<VM> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

impl <VM: VMBinding> ProcessEdgesWork for SanityGCProcessEdges<VM> {
    type VM = VM;
    const OVERWRITE_REFERENCE: bool = false;
    fn new(edges: Vec<Address>, _roots: bool) -> Self {
        Self { base: ProcessEdgesBase::new(edges), ..Default::default() }
    }

    #[inline]
    fn process_edge(&mut self, slot: Address) {
        let object = unsafe { slot.load::<ObjectReference>() };
        assert!(!self.plan().nursery.in_space(object)
            "Invalid edge: {:?} -> {:?}", slot, object
        );
        assert!(!self.plan().fromspace().in_space(object)
            "Invalid edge: {:?} -> {:?}", slot, object
        );
        self.trace_object(object);
    }

    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        if self.plan().tospace().in_space(object) {
            return self.plan().tospace().trace_mark_object(self, object);
        }
        unreachable!()
    }
}