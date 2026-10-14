//! Static process/thread budget, never a sampled CPU feedback controller.
#[derive(Debug, Clone, Copy)]
pub struct CpuBudget {
    pub logical_cores: usize,
    pub half_machine_slots: usize,
    pub asr_threads: u32,
    pub shared_encoder_slot: bool,
}
impl CpuBudget {
    pub fn current() -> Self {
        Self::for_cores(std::thread::available_parallelism().map_or(1, usize::from))
    }
    pub fn for_cores(logical_cores: usize) -> Self {
        let cores = logical_cores.max(1);
        let slots = (cores / 2).max(1);
        Self {
            logical_cores: cores,
            half_machine_slots: slots,
            asr_threads: slots.saturating_sub(2).clamp(1, 3) as u32,
            shared_encoder_slot: slots < 3,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_half_machine_budget() {
        let b = CpuBudget::for_cores(8);
        assert_eq!(b.asr_threads, 2);
        assert!(!b.shared_encoder_slot);
        assert!(CpuBudget::for_cores(4).shared_encoder_slot);
        assert_eq!(CpuBudget::for_cores(12).asr_threads, 3);
    }
}
