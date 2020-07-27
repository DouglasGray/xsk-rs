pub struct FrameCount(u32);
pub struct FrameSize(u32);
pub struct FrameHeadroom(u32);
pub struct FillQueueSize(u32);
pub struct CompQueueSize(u32);

pub struct UmemConfig {
    frame_count: u32,
    frame_size: u32,
    fill_queue_size: u32,
    comp_queue_size: u32,
    frame_headroom: u32,
    use_huge_pages: bool,
}

impl UmemConfig {
    fn new(
        frame_count: FrameCount,
        frame_size: FrameSize,
        fill_queue_size: FillQueueSize,
        comp_queue_size: CompQueueSize,
        use_huge_pages: bool,
    ) {
        let frame_count = frame_count.0.next_power_of_two() as usize;
        let frame_size = frame_size.0.next_power_of_two() as usize;
        let frame
        unimplemented!();
    }
}
