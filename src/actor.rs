use std::fs::File;
use unreal_asset::{
    cast,
    error::Error,
    exports::{Export, ExportBaseTrait, ExportNormalTrait},
    properties::{Property, PropertyDataTrait},
    reader::archive_trait::ArchiveTrait,
    types::{fname::FName, PackageIndex},
    Asset,
};

mod delete;
mod duplicate;
mod transform;
mod transplant;
mod ui;

pub enum DrawType {
    Mesh(String),
    Cube,
}

pub struct Actor {
    export: usize,
    transform: usize,
    pub name: String,
    pub class: String,
    pub draw_type: DrawType,
}

impl Actor {
    fn index(&self) -> PackageIndex {
        PackageIndex::new(self.export as i32 + 1)
    }

    pub fn new(asset: &crate::Asset, package: PackageIndex) -> Result<Self, Error> {
        if package.index == 0 {
            return Err(Error::invalid_package_index(
                "actor was null reference".to_string(),
            ));
        }
        let export = package.index as usize - 1;
        let Some(ex) = asset.get_export(package) else {
            return Err(Error::invalid_package_index(format!(
                "failed to find actor at index {}",
                package.index
            )));
        };
        let Some(norm) = ex.get_normal_export() else {
            return Err(Error::no_data(format!(
                "actor at index {} failed to parse",
                package.index
            )));
        };
        let name = match asset.get_engine_version()
            >= unreal_asset::engine_version::EngineVersion::VER_UE5_1
        {
            true => {
                let len = i32::from_le_bytes(norm.extras[8..12].try_into().unwrap()) as usize;
                String::from_utf8(norm.extras[12..12 + len].to_vec()).unwrap()
            }
            false => norm.base_export.object_name.get_owned_content(),
        };
        let class = asset
            .get_import(norm.base_export.class_index)
            .map(|import| import.object_name.get_owned_content())
            .unwrap_or_default();
        let draw_type = norm
            .base_export
            .create_before_serialization_dependencies
            .iter()
            .filter_map(|i| asset.get_export(*i))
            .filter_map(Export::get_normal_export)
            .find(|i| {
                asset
                    .get_import(i.get_base_export().class_index)
                    .filter(|import| import.object_name == "StaticMeshComponent")
                    .is_some()
            })
            .and_then(|norm| {
                norm.properties.iter().find_map(|prop| {
                    cast!(Property, ObjectProperty, prop)
                        .filter(|prop| prop.get_name() == "StaticMesh")
                })
            })
            .and_then(|obj| asset.get_import(obj.value))
            .and_then(|import| asset.get_import(import.outer_index))
            .map_or(DrawType::Cube, |path| {
                DrawType::Mesh(path.object_name.get_owned_content())
            });
        // normally these are further back so reversed should be a bit faster
        for prop in norm.properties.iter().rev() {
            match prop.get_name().get_owned_content().as_str() {
                // of course this wouldn't be able to be detected if all transforms were left default
                "RelativeLocation" | "RelativeRotation" | "RelativeScale3D" => {
                    return Ok(Self {
                        export,
                        transform: export,
                        name,
                        class,
                        draw_type,
                    })
                }
                "RootComponent" => {
                    if let Property::ObjectProperty(obj) = prop {
                        if obj.value.is_export() {
                            return Ok(Self {
                                export,
                                transform: obj.value.index as usize - 1,
                                name,
                                class,
                                draw_type,
                            });
                        }
                    }
                }
                _ => continue,
            }
        }
        norm.base_export.object_name.get_content(|name| {
            Err(Error::no_data(format!(
                "couldn't find transform component for {name}",
            )))
        })
    }

    fn get_actor_indexes(&self, asset: &Asset<std::io::BufReader<File>>) -> Vec<PackageIndex> {
        // get references to all the actor's children
        let mut child_indexes: Vec<PackageIndex> = asset.asset_data.exports[self.export]
            .get_base_export()
            .create_before_serialization_dependencies
            .iter()
            .filter(|dep| dep.is_export())
            // dw PackageIndex is just a wrapper around i32 which is cloned by default anyway
            .cloned()
            .collect();
        if let Some(level) = asset
            .asset_data
            .exports
            .iter()
            .find_map(|ex| unreal_asset::cast!(Export, LevelExport, ex))
        {
            let actors: Vec<_> = child_indexes
                .iter()
                .enumerate()
                .rev()
                .filter_map(|(i, child)| level.actors.contains(&child).then_some(i))
                .collect();
            for i in actors {
                child_indexes.remove(i);
            }
        }
        // add the top-level actor reference
        child_indexes.insert(0, self.index());
        child_indexes
    }

    /// gets all exports related to the given actor
    fn get_actor_exports(
        &self,
        asset: &Asset<std::io::BufReader<File>>,
        offset: usize,
    ) -> Vec<Export> {
        let child_indexes = self.get_actor_indexes(asset);
        // get all the exports from those indexes
        let mut children: Vec<Export> = child_indexes
            .iter()
            .filter_map(|index| asset.get_export(*index))
            // i'm pretty sure i have to clone here so i can modify then insert data
            .cloned()
            .collect();

        let package_offset = (offset + 1) as i32;
        // update export references to what they will be once added
        for (i, child_index) in child_indexes.into_iter().enumerate() {
            for child in children.iter_mut() {
                on_export_refs(child, |index| {
                    if index == &child_index {
                        index.index = package_offset + i as i32;
                    }
                });
            }
        }
        children
    }
}

/// gets all actor exports within a map (all exports direct children of PersistentLevel)
pub fn get_actors(asset: &crate::Asset) -> Vec<PackageIndex> {
    match asset
        .asset_data
        .exports
        .iter()
        .find_map(|ex| cast!(Export, LevelExport, ex))
    {
        Some(level) => level
            .actors
            .iter()
            .filter(|index| index.is_export())
            .copied()
            .collect(),
        None => Vec::new(),
    }
}

/// creates and assigns a unique name
fn give_unique_name(orig: &mut FName, asset: &mut crate::Asset) {
    // for the cases where the number is unnecessary
    let mut name = orig.get_owned_content();
    if asset.search_name_reference(&name).is_none() {
        *orig = asset.add_fname(&name);
        return;
    }
    let mut id: u16 = match name.rfind(|ch: char| ch.to_digit(10).is_none()) {
        Some(index) if index != name.len() - 1 => {
            name.drain(index + 1..).collect::<String>().parse().unwrap()
        }
        _ => 1,
    };
    while asset
        .search_name_reference(&format!("{}{}", &name, id))
        .is_some()
    {
        id += 1;
    }
    *orig = asset.add_fname(&(name + &id.to_string()))
}

/// on all possible export references
fn on_export_refs(export: &mut Export, mut func: impl FnMut(&mut PackageIndex)) {
    fn base(base: &mut unreal_asset::exports::BaseExport, mut func: impl FnMut(&mut PackageIndex)) {
        base.create_before_create_dependencies
            .iter_mut()
            .for_each(&mut func);
        base.create_before_serialization_dependencies
            .iter_mut()
            .for_each(&mut func);
        base.serialization_before_create_dependencies
            .iter_mut()
            .for_each(&mut func);
        func(&mut base.outer_index);
    }
    fn norm(
        norm: &mut unreal_asset::exports::NormalExport,
        mut func: impl FnMut(&mut PackageIndex),
    ) {
        for prop in norm.properties.iter_mut() {
            on_prop_refs(prop, &mut func);
        }
        base(&mut norm.base_export, func);
    }
    fn struc(
        struc: &mut unreal_asset::exports::StructExport,
        mut func: impl FnMut(&mut PackageIndex),
    ) {
        if let Some(field) = struc.field.next.as_mut() {
            func(field)
        }
        func(&mut struc.super_struct);
        struc.children.iter_mut().for_each(&mut func);
        for fprop in struc.loaded_properties.iter_mut() {
            if let unreal_asset::fproperty::FProperty::FEnumProperty(en) = fprop {
                func(&mut en.enum_value)
            }
        }
        if let Some(kismet) = struc.script_bytecode.as_mut() {
            use unreal_asset::KismetExpression::*;
            fn pointer(
                pointer: &mut unreal_asset::kismet::KismetPropertyPointer,
                func: &mut impl FnMut(&mut PackageIndex),
            ) {
                if let Some(old) = pointer.old.as_mut() {
                    func(old)
                }
                if let Some(new) = pointer.new.as_mut() {
                    func(&mut new.resolved_owner)
                }
            }
            fn expr(
                inst: &mut unreal_asset::KismetExpression,
                func: &mut impl FnMut(&mut PackageIndex),
            ) {
                match inst {
                    ExLocalVariable(ex) => pointer(&mut ex.variable, func),
                    ExInstanceVariable(ex) => pointer(&mut ex.variable, func),
                    ExDefaultVariable(ex) => pointer(&mut ex.variable, func),
                    ExReturn(ex) => expr(&mut ex.return_expression, func),
                    // ExJump(ex) => todo!(),
                    ExJumpIfNot(ex) => expr(&mut ex.boolean_expression, func),
                    ExAssert(ex) => expr(&mut ex.assert_expression, func),
                    // ExNothing(ex) => todo!(),
                    ExLet(ex) => {
                        pointer(&mut ex.value, func);
                        expr(&mut ex.variable, func);
                        expr(&mut ex.expression, func);
                    }
                    ExClassContext(ex) => {
                        expr(&mut ex.object_expression, func);
                        pointer(&mut ex.r_value_pointer, func);
                        expr(&mut ex.context_expression, func);
                    }
                    ExMetaCast(ex) => {
                        func(&mut ex.class_ptr);
                        expr(&mut ex.target_expression, func);
                    }
                    ExLetBool(ex) => {
                        expr(&mut ex.variable_expression, func);
                        expr(&mut ex.assignment_expression, func);
                    }
                    // ExEndParmValue(ex) => todo!(),
                    // ExEndFunctionParms(ex) => todo!(),
                    // ExSelf(ex) => todo!(),
                    ExSkip(ex) => expr(&mut ex.skip_expression, func),
                    ExContext(ex) => {
                        expr(&mut ex.object_expression, func);
                        pointer(&mut ex.r_value_pointer, func);
                        expr(&mut ex.context_expression, func);
                    }
                    ExContextFailSilent(ex) => {
                        expr(&mut ex.object_expression, func);
                        pointer(&mut ex.r_value_pointer, func);
                        expr(&mut ex.context_expression, func);
                    }
                    ExVirtualFunction(ex) => {
                        for param in ex.parameters.iter_mut() {
                            expr(param, func)
                        }
                    }
                    ExFinalFunction(ex) => {
                        func(&mut ex.stack_node);
                        for param in ex.parameters.iter_mut() {
                            expr(param, func)
                        }
                    }
                    // ExIntConst(ex) => todo!(),
                    // ExFloatConst(ex) => todo!(),
                    // ExStringConst(ex) => todo!(),
                    // ExObjectConst(ex) => todo!(),
                    // ExNameConst(ex) => todo!(),
                    // ExRotationConst(ex) => todo!(),
                    // ExVectorConst(ex) => todo!(),
                    // ExByteConst(ex) => todo!(),
                    // ExIntZero(ex) => todo!(),
                    // ExIntOne(ex) => todo!(),
                    // ExTrue(ex) => todo!(),
                    // ExFalse(ex) => todo!(),
                    // ExTextConst(ex) => todo!(),
                    // ExNoObject(ex) => todo!(),
                    // ExTransformConst(ex) => todo!(),
                    // ExIntConstByte(ex) => todo!(),
                    // ExNoInterface(ex) => todo!(),
                    ExDynamicCast(ex) => {
                        func(&mut ex.class_ptr);
                        expr(&mut ex.target_expression, func);
                    }
                    ExStructConst(ex) => {
                        func(&mut ex.struct_value);
                        for val in ex.value.iter_mut() {
                            expr(val, func)
                        }
                    }
                    // ExEndStructConst(ex) => todo!(),
                    ExSetArray(ex) => {
                        if let Some(prop) = ex.assigning_property.as_mut() {
                            expr(prop, func)
                        }
                        if let Some(prop) = ex.array_inner_prop.as_mut() {
                            func(prop)
                        }
                        for elem in ex.elements.iter_mut() {
                            expr(elem, func)
                        }
                    }
                    // ExEndArray(ex) => todo!(),
                    ExPropertyConst(ex) => pointer(&mut ex.property, func),
                    // ExUnicodeStringConst(ex) => todo!(),
                    // ExInt64Const(ex) => todo!(),
                    // ExUInt64Const(ex) => todo!(),
                    ExPrimitiveCast(ex) => expr(&mut ex.target, func),
                    ExSetSet(ex) => {
                        expr(&mut ex.set_property, func);
                        for elem in ex.elements.iter_mut() {
                            expr(elem, func)
                        }
                    }
                    // ExEndSet(ex) => todo!(),
                    ExSetMap(ex) => {
                        expr(&mut ex.map_property, func);
                        for elem in ex.elements.iter_mut() {
                            expr(elem, func)
                        }
                    }
                    // ExEndMap(ex) => todo!(),
                    ExSetConst(ex) => {
                        pointer(&mut ex.inner_property, func);
                        for elem in ex.elements.iter_mut() {
                            expr(elem, func);
                        }
                    }
                    // ExEndSetConst(ex) => todo!(),
                    ExMapConst(ex) => {
                        pointer(&mut ex.key_property, func);
                        pointer(&mut ex.value_property, func);
                        for elem in ex.elements.iter_mut() {
                            expr(elem, func);
                        }
                    }
                    // ExEndMapConst(ex) => todo!(),
                    ExStructMemberContext(ex) => {
                        pointer(&mut ex.struct_member_expression, func);
                        expr(&mut ex.struct_expression, func);
                    }
                    ExLetMulticastDelegate(ex) => {
                        expr(&mut ex.variable_expression, func);
                        expr(&mut ex.assignment_expression, func);
                    }
                    ExLetDelegate(ex) => {
                        expr(&mut ex.variable_expression, func);
                        expr(&mut ex.assignment_expression, func);
                    }
                    ExLocalVirtualFunction(ex) => {
                        for param in ex.parameters.iter_mut() {
                            expr(param, func)
                        }
                    }
                    ExLocalFinalFunction(ex) => {
                        func(&mut ex.stack_node);
                        for param in ex.parameters.iter_mut() {
                            expr(param, func)
                        }
                    }
                    ExLocalOutVariable(ex) => pointer(&mut ex.variable, func),
                    // ExDeprecatedOp4A(ex) => todo!(),
                    // ExInstanceDelegate(ex) => todo!(),
                    // ExPushExecutionFlow(ex) => todo!(),
                    // ExPopExecutionFlow(ex) => todo!(),
                    ExComputedJump(ex) => expr(&mut ex.code_offset_expression, func),
                    ExPopExecutionFlowIfNot(ex) => expr(&mut ex.boolean_expression, func),
                    // ExBreakpoint(ex) => todo!(),
                    ExInterfaceContext(ex) => expr(&mut ex.interface_value, func),
                    ExObjToInterfaceCast(ex) => {
                        func(&mut ex.class_ptr);
                        expr(&mut ex.target, func);
                    }
                    // ExEndOfScript(ex) => todo!(),
                    ExCrossInterfaceCast(ex) => {
                        func(&mut ex.class_ptr);
                        expr(&mut ex.target, func);
                    }
                    ExInterfaceToObjCast(ex) => {
                        func(&mut ex.class_ptr);
                        expr(&mut ex.target, func);
                    }
                    // ExWireTracepoint(ex) => todo!(),
                    // ExSkipOffsetConst(ex) => todo!(),
                    ExAddMulticastDelegate(ex) => {
                        expr(&mut ex.delegate, func);
                        expr(&mut ex.delegate_to_add, func);
                    }
                    ExClearMulticastDelegate(ex) => expr(&mut ex.delegate_to_clear, func),
                    // ExTracepoint(ex) => todo!(),
                    ExLetObj(ex) => {
                        expr(&mut ex.variable_expression, func);
                        expr(&mut ex.assignment_expression, func);
                    }
                    ExLetWeakObjPtr(ex) => {
                        expr(&mut ex.variable_expression, func);
                        expr(&mut ex.assignment_expression, func);
                    }
                    ExBindDelegate(ex) => {
                        expr(&mut ex.delegate, func);
                        expr(&mut ex.object_term, func);
                    }
                    ExRemoveMulticastDelegate(ex) => {
                        expr(&mut ex.delegate, func);
                        expr(&mut ex.delegate_to_add, func);
                    }
                    ExCallMulticastDelegate(ex) => {
                        func(&mut ex.stack_node);
                        for param in ex.parameters.iter_mut() {
                            expr(param, func)
                        }
                        expr(&mut ex.delegate, func);
                    }
                    ExLetValueOnPersistentFrame(ex) => {
                        pointer(&mut ex.destination_property, func);
                        expr(&mut ex.assignment_expression, func);
                    }
                    ExArrayConst(ex) => {
                        pointer(&mut ex.inner_property, func);
                        for elem in ex.elements.iter_mut() {
                            expr(elem, func);
                        }
                    }
                    // ExEndArrayConst(ex) => todo!(),
                    ExSoftObjectConst(ex) => expr(&mut ex.value, func),
                    ExCallMath(ex) => {
                        func(&mut ex.stack_node);
                        for param in ex.parameters.iter_mut() {
                            expr(param, func)
                        }
                    }
                    ExSwitchValue(ex) => {
                        expr(&mut ex.index_term, func);
                        expr(&mut ex.default_term, func);
                        for case in ex.cases.iter_mut() {
                            expr(&mut case.case_index_value_term, func);
                            expr(&mut case.case_term, func);
                        }
                    }
                    // ExInstrumentationEvent(ex) => todo!(),
                    ExArrayGetByRef(ex) => {
                        expr(&mut ex.array_variable, func);
                        expr(&mut ex.array_index, func);
                    }
                    ExClassSparseDataVariable(ex) => pointer(&mut ex.variable, func),
                    ExFieldPathConst(ex) => expr(&mut ex.value, func),
                    _ => (),
                }
            }
            for inst in kismet.iter_mut() {
                expr(inst, &mut func)
            }
        }
        norm(&mut struc.normal_export, func)
    }
    match export {
        Export::BaseExport(bas) => base(bas, func),
        Export::ClassExport(class) => {
            class.func_map.values_mut().for_each(&mut func);
            func(&mut class.class_within);
            for interface in class.interfaces.iter_mut() {
                func(&mut interface.class)
            }
            func(&mut class.class_generated_by);
            func(&mut class.class_default_object);
            struc(&mut class.struct_export, func);
        }
        Export::EnumExport(en) => norm(&mut en.normal_export, func),
        Export::LevelExport(level) => {
            level.actors.iter_mut().for_each(&mut func);
            func(&mut level.model);
            level.model_components.iter_mut().for_each(&mut func);
            func(&mut level.level_script);
            func(&mut level.nav_list_start);
            func(&mut level.nav_list_end);
            norm(&mut level.normal_export, func)
        }
        Export::NormalExport(normal) => norm(normal, func),
        Export::PropertyExport(prop) => {
            use unreal_asset::uproperty::UProperty::*;
            fn generic(
                gen: &mut unreal_asset::uproperty::UGenericProperty,
                func: &mut impl FnMut(&mut PackageIndex),
            ) {
                if let Some(next) = gen.u_field.next.as_mut() {
                    func(next)
                }
            }
            match &mut prop.property {
                UGenericProperty(gen) => generic(gen, &mut func),
                UEnumProperty(prop) => {
                    func(&mut prop.value);
                    func(&mut prop.underlying_prop);
                    generic(&mut prop.generic_property, &mut func);
                }
                UArrayProperty(prop) => {
                    func(&mut prop.inner);
                    generic(&mut prop.generic_property, &mut func);
                }
                USetProperty(prop) => {
                    func(&mut prop.element_prop);
                    generic(&mut prop.generic_property, &mut func)
                }
                UObjectProperty(prop) => {
                    func(&mut prop.property_class);
                    generic(&mut prop.generic_property, &mut func);
                }
                USoftObjectProperty(prop) => {
                    func(&mut prop.property_class);
                    generic(&mut prop.generic_property, &mut func);
                }
                ULazyObjectProperty(prop) => {
                    func(&mut prop.property_class);
                    generic(&mut prop.generic_property, &mut func);
                }
                UClassProperty(prop) => {
                    func(&mut prop.property_class);
                    func(&mut prop.meta_class);
                    generic(&mut prop.generic_property, &mut func);
                }
                USoftClassProperty(prop) => {
                    func(&mut prop.property_class);
                    func(&mut prop.meta_class);
                    generic(&mut prop.generic_property, &mut func);
                }
                UDelegateProperty(prop) => {
                    func(&mut prop.signature_function);
                    generic(&mut prop.generic_property, &mut func);
                }
                UMulticastDelegateProperty(prop) => {
                    func(&mut prop.signature_function);
                    generic(&mut prop.generic_property, &mut func);
                }
                UMulticastInlineDelegateProperty(prop) => {
                    func(&mut prop.signature_function);
                    generic(&mut prop.generic_property, &mut func);
                }
                UInterfaceProperty(prop) => {
                    func(&mut prop.interface_class);
                    generic(&mut prop.generic_property, &mut func);
                }
                UMapProperty(prop) => {
                    func(&mut prop.key_prop);
                    func(&mut prop.value_prop);
                    generic(&mut prop.generic_property, &mut func);
                }
                UBoolProperty(prop) => generic(&mut prop.generic_property, &mut func),
                UByteProperty(prop) => {
                    func(&mut prop.enum_value);
                    generic(&mut prop.generic_property, &mut func);
                }
                UStructProperty(prop) => {
                    func(&mut prop.struct_value);
                    generic(&mut prop.generic_property, &mut func);
                }
                UDoubleProperty(prop) => generic(&mut prop.generic_property, &mut func),
                UFloatProperty(prop) => generic(&mut prop.generic_property, &mut func),
                UIntProperty(prop) => generic(&mut prop.generic_property, &mut func),
                UInt8Property(prop) => generic(&mut prop.generic_property, &mut func),
                UInt16Property(prop) => generic(&mut prop.generic_property, &mut func),
                UInt64Property(prop) => generic(&mut prop.generic_property, &mut func),
                UUInt8Property(prop) => generic(&mut prop.generic_property, &mut func),
                UUInt16Property(prop) => generic(&mut prop.generic_property, &mut func),
                UUInt64Property(prop) => generic(&mut prop.generic_property, &mut func),
                UNameProperty(prop) => generic(&mut prop.generic_property, &mut func),
                UStrProperty(prop) => generic(&mut prop.generic_property, &mut func),
            }
            norm(&mut prop.normal_export, func);
        }
        Export::RawExport(_) => (),
        Export::StringTableExport(table) => norm(&mut table.normal_export, func),
        Export::StructExport(str) => struc(str, func),
        Export::UserDefinedStructExport(uds) => {
            for prop in uds.default_struct_instance.iter_mut() {
                on_prop_refs(prop, &mut func)
            }
            struc(&mut uds.struct_export, func);
        }
        Export::FunctionExport(fun) => struc(&mut fun.struct_export, func),
        Export::DataTableExport(table) => {
            for str in table.table.data.iter_mut() {
                for prop in str.value.iter_mut() {
                    on_prop_refs(prop, &mut func)
                }
            }
            norm(&mut table.normal_export, func);
        }
        Export::WorldExport(world) => {
            func(&mut world.persistent_level);
            world.extra_objects.iter_mut().for_each(&mut func);
            world.streaming_levels.iter_mut().for_each(&mut func);
            norm(&mut world.normal_export, func);
        }
    }
}

fn on_props(prop: &mut Property, func: &mut impl FnMut(&mut Property)) {
    match prop {
        Property::ArrayProperty(arr) => {
            for entry in arr.value.iter_mut() {
                on_props(entry, func);
            }
        }
        Property::MapProperty(map) => {
            for val in map.value.values_mut() {
                on_props(val, func);
            }
        }
        Property::SetProperty(set) => {
            for entry in set.value.value.iter_mut() {
                on_props(entry, func);
            }
            for entry in set.removed_items.value.iter_mut() {
                on_props(entry, func);
            }
        }
        Property::StructProperty(struc) => {
            for entry in struc.value.iter_mut() {
                on_props(entry, func);
            }
        }
        prop => func(prop),
    }
}

/// on any possible references stashed away in properties
fn on_prop_refs(prop: &mut Property, func: &mut impl FnMut(&mut PackageIndex)) {
    on_props(prop, &mut |prop| match prop {
        Property::ObjectProperty(obj) => {
            func(&mut obj.value);
        }
        Property::DelegateProperty(del) => func(&mut del.value.object),
        Property::MulticastDelegateProperty(del) => {
            for delegate in del.value.iter_mut() {
                func(&mut delegate.object)
            }
        }
        Property::MulticastSparseDelegateProperty(del) => {
            for delegate in del.value.iter_mut() {
                func(&mut delegate.object)
            }
        }
        Property::MulticastInlineDelegateProperty(del) => {
            for delegate in del.value.iter_mut() {
                func(&mut delegate.object)
            }
        }
        _ => (),
    })
}
