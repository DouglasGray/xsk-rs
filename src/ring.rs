use libbpf_sys::{xsk_ring_cons, xsk_ring_prod};

#[derive(Default)]
pub struct XskRingCons(pub xsk_ring_cons);

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

unsafe impl Send for XskRingCons {}

#[derive(Default)]
pub struct XskRingProd(pub xsk_ring_prod);

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

unsafe impl Send for XskRingProd {}