use unreal_asset::{
    cast,
    exports::{Export, ExportBaseTrait},
    types::PackageIndex,
};

impl super::Actor {
    /// delete an actor from a map
    pub fn delete(&self, actors: &mut Vec<Self>, map: &mut crate::Asset) {
        let val = PackageIndex::new(self.export as i32 + 1);
        if let Some(level) = map
            .asset_data
            .exports
            .iter_mut()
            .find_map(|ex| cast!(Export, LevelExport, ex))
        {
            level
                .actors
                .remove(level.actors.iter().position(|i| i == &val).unwrap());
            let pos = level
                .get_base_export()
                .create_before_serialization_dependencies
                .iter()
                .position(|i| i == &val)
                .unwrap();
            level
                .get_base_export_mut()
                .create_before_serialization_dependencies
                .remove(pos);
        }
        let mut refs = self.get_actor_indexes(map);
        // ensures that deleting the export doesn't change other indexes
        refs.sort_unstable_by_key(|key| std::cmp::Reverse(key.index));
        for index in refs {
            for export in map.asset_data.exports.iter_mut() {
                super::on_export_refs(export, |i| match i.index.cmp(&index.index) {
                    std::cmp::Ordering::Less => (),
                    std::cmp::Ordering::Equal => i.index = 0,
                    std::cmp::Ordering::Greater => i.index -= 1,
                })
            }
            for actor in actors.iter_mut() {
                let cmp = |i: &mut usize| match (*i as i32).cmp(&index.index) {
                    std::cmp::Ordering::Less => (),
                    std::cmp::Ordering::Equal => *i = 0,
                    std::cmp::Ordering::Greater => *i -= 1,
                };
                cmp(&mut actor.export);
                cmp(&mut actor.transform);
            }
            map.asset_data.exports.remove(index.index as usize - 1);
        }
    }
}
