use crate::vanilla_parity::{dense_dump, lattice_dumps, overworld_router};
use bevy_math::IVec3;
use mcrs_minecraft_worldgen_density::program::Workspace;
use mcrs_minecraft_worldgen_density::router::FINAL_DENSITY;
use mcrs_minecraft_worldgen_noise::sample_grid::SampleGrid;

mod exhaustive {
    use super::*;

    #[test]
    fn the_density_fixture_holds_fifteen_lattice_dumps() {
        assert_eq!(
            lattice_dumps().len(),
            15,
            "expected the fifteen lattice dumps"
        );
    }

    #[test]
    fn the_dense_dump_covers_a_whole_chunk_column() {
        let dump = dense_dump();
        let v = dump
            .volumes
            .iter()
            .find(|v| v.name == "final_density")
            .expect("dense dump holds final_density");
        assert_eq!(v.values.len(), 98_304, "16x384x16 dense");
    }

    /// The schedule decides per fill, not per column: an arm's cone is skipped only
    /// when no position in the whole volume selects it. Over a box as large as a
    /// chunk that is a weaker predicate than a per-column one, so the share it skips
    /// is asserted rather than trusted.
    #[test]
    fn the_branch_schedule_skips_a_real_share_of_an_overworld_chunk() {
        let router = overworld_router(42);
        let volume = SampleGrid::dense(IVec3::new(16, 384, 16), IVec3::new(0, -64, 0));
        let mut out = vec![f32::NAN; volume.len()];
        let mut ws = Workspace::new();
        ws.take_count();
        router
            .program
            .fill(&mut ws, &volume, FINAL_DENSITY, &mut out);

        let count = ws.take_count();
        let total = count.evaluated + count.skipped;
        let share = count.skipped as f64 / total as f64;
        println!(
            "final_density over 16x384x16: {} of {total} node evaluations skipped ({:.1}%)",
            count.skipped,
            share * 100.0
        );
        assert!(
            share > 0.15,
            "the schedule skipped only {:.1}% of {total} evaluations",
            share * 100.0
        );
    }
}
