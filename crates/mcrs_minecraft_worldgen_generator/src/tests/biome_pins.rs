use super::biome_parity::{BLOCKS, read_dump};

mod exhaustive {
    use super::*;

    #[test]
    fn every_dumped_biome_section_holds_a_full_section() {
        for column in read_dump() {
            let (chunk_x, chunk_z) = (column.chunk_x, column.chunk_z);
            for section in &column.sections {
                assert_eq!(
                    section.len(),
                    BLOCKS,
                    "a section of chunk {chunk_x},{chunk_z} is short"
                );
            }
        }
    }
}
