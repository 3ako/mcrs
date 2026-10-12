use super::surface_parity::{DUMPS, read_dump};

mod exhaustive {
    use super::*;

    #[test]
    fn every_surface_dump_was_taken_with_overworld_settings() {
        for name in DUMPS {
            assert_eq!(read_dump(name).settings, "minecraft:overworld");
        }
    }
}
