use crate::structure_placement_parity::read_dump;

mod exhaustive {
    use super::*;

    #[test]
    fn the_placement_dump_covers_every_cell_of_both_squares() {
        let cells: usize = read_dump()
            .iter()
            .flat_map(|seed| &seed.dims)
            .flat_map(|dim| &dim.sets)
            .flat_map(|set| &set.grids)
            .map(|grid| grid.cells.len())
            .sum();
        assert_eq!(cells, 5 * 21 * (49 * 49 + 25 * 25));
    }
}
