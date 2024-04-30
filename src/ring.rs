use std::ptr;

use libxdp_sys::{xsk_ring_cons, xsk_ring_prod};

#[derive(Debug)]
pub struct XskRingCons(xsk_ring_cons);

impl XskRingCons {
    pub fn as_mut(&mut self) -> &mut xsk_ring_cons {
        &mut self.0
    }

    pub fn as_ref(&self) -> &xsk_ring_cons {
        &self.0
    }

    pub fn is_ring_null(&self) -> bool {
        self.0.ring.is_null()
    }
}

impl Default for XskRingCons {
    fn default() -> Self {
        Self(xsk_ring_cons {
            cached_prod: 0,
            cached_cons: 0,
            mask: 0,
            size: 0,
            producer: ptr::null_mut(),
            consumer: ptr::null_mut(),
            ring: ptr::null_mut(),
            flags: ptr::null_mut(),
        })
    }
}

unsafe impl Send for XskRingCons {}

#[derive(Debug)]
pub struct XskRingProd(xsk_ring_prod);

impl XskRingProd {
    pub fn as_mut(&mut self) -> &mut xsk_ring_prod {
        &mut self.0
    }

    pub fn as_ref(&self) -> &xsk_ring_prod {
        &self.0
    }

    pub fn is_ring_null(&self) -> bool {
        self.0.ring.is_null()
    }
}

impl Default for XskRingProd {
    fn default() -> Self {
        Self(xsk_ring_prod {
            cached_prod: 0,
            cached_cons: 0,
            mask: 0,
            size: 0,
            producer: ptr::null_mut(),
            consumer: ptr::null_mut(),
            ring: ptr::null_mut(),
            flags: ptr::null_mut(),
        })
    }
}

unsafe impl Send for XskRingProd {}
