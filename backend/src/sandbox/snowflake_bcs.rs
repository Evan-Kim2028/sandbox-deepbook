//! JSON to BCS Reconstruction Utility
//!
//! This module converts Snowflake's OBJECT_JSON (decoded Move object data) back to
//! BCS bytes using struct layouts extracted from Move bytecode.
//!
//! Copied from sui-sandbox examples/common/snowflake_bcs.rs

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use move_core_types::account_address::AccountAddress;
use serde_json::Value as JsonValue;

// Import from sui-sandbox-core utilities - these are public but not re-exported
use sui_sandbox_core::utilities::generic_patcher::{
    BcsEncoder, DynamicValue, LayoutRegistry, MoveType, StructLayout,
};

/// Reconstructs BCS bytes from Snowflake's OBJECT_JSON using bytecode layouts.
pub struct JsonToBcsConverter {
    layout_registry: LayoutRegistry,
}

impl JsonToBcsConverter {
    /// Create a new converter with an empty layout registry.
    pub fn new() -> Self {
        Self {
            layout_registry: LayoutRegistry::new(),
        }
    }

    /// Add modules from raw bytecode bytes.
    pub fn add_modules_from_bytes(&mut self, bytecode_list: &[Vec<u8>]) -> Result<()> {
        use move_binary_format::CompiledModule;
        for bytecode in bytecode_list {
            let module = CompiledModule::deserialize_with_defaults(bytecode)
                .map_err(|e| anyhow!("Failed to deserialize module: {:?}", e))?;
            self.layout_registry.add_modules(std::iter::once(&module));
        }
        Ok(())
    }

    /// Convert OBJECT_JSON to BCS bytes.
    ///
    /// # Arguments
    /// * `type_str` - The full Sui type string (e.g., "0x97d...::margin_manager::MarginManager<...>")
    /// * `object_json` - The decoded object data from Snowflake's OBJECT_JSON column
    ///
    /// # Returns
    /// The BCS-encoded bytes that can be loaded into the VM.
    pub fn convert(&mut self, type_str: &str, object_json: &JsonValue) -> Result<Vec<u8>> {
        // Get the struct layout AND type args from bytecode
        let (layout, type_args) = self
            .layout_registry
            .get_layout_with_type_args(type_str)
            .ok_or_else(|| anyhow!("Could not find layout for type: {}", type_str))?;

        // Debug-only: print layout field order for slice conversions.
        if type_str.contains("Slice") {
            tracing::debug!(
                "convert: type={}, layout.fields={:?}",
                type_str,
                layout.fields.iter().map(|f| &f.name).collect::<Vec<_>>()
            );
        }

        // Convert JSON to DynamicValue following the layout
        let value = self.json_to_dynamic_value_with_type_args(object_json, &layout, &type_args)?;

        // Encode to BCS
        let mut encoder = BcsEncoder::new();
        let bcs_bytes = encoder
            .encode(&value)
            .with_context(|| format!("Failed to encode {} to BCS", type_str))?;

        Ok(bcs_bytes)
    }

    /// Substitute type parameters in a MoveType using the provided type arguments.
    fn substitute_type_params(&self, move_type: &MoveType, type_args: &[MoveType]) -> MoveType {
        match move_type {
            MoveType::TypeParameter(idx) => {
                if (*idx as usize) < type_args.len() {
                    type_args[*idx as usize].clone()
                } else {
                    move_type.clone()
                }
            }
            MoveType::Vector(inner) => {
                MoveType::Vector(Box::new(self.substitute_type_params(inner, type_args)))
            }
            MoveType::Struct {
                address,
                module,
                name,
                type_args: nested_type_args,
            } => MoveType::Struct {
                address: *address,
                module: module.clone(),
                name: name.clone(),
                type_args: nested_type_args
                    .iter()
                    .map(|t| self.substitute_type_params(t, type_args))
                    .collect(),
            },
            _ => move_type.clone(),
        }
    }

    /// Convert a JSON object to DynamicValue using the struct layout with type parameter substitution.
    fn json_to_dynamic_value_with_type_args(
        &mut self,
        json: &JsonValue,
        layout: &StructLayout,
        type_args: &[MoveType],
    ) -> Result<DynamicValue> {
        let json_obj = json
            .as_object()
            .ok_or_else(|| anyhow!("Expected JSON object for struct {}", layout.name))?;

        let mut fields = Vec::new();

        // Process fields in the ORDER defined by the struct layout (critical for BCS!)
        for field_layout in &layout.fields {
            let field_name = &field_layout.name;
            let field_type = self.substitute_type_params(&field_layout.field_type, type_args);

            let json_value = json_obj.get(field_name).ok_or_else(|| {
                anyhow!(
                    "Missing field '{}' in JSON for struct {}",
                    field_name,
                    layout.name
                )
            })?;

            let value = self.convert_field(json_value, &field_type, field_name)?;
            fields.push((field_name.clone(), value));
        }

        Ok(DynamicValue::Struct {
            type_name: layout.name.clone(),
            fields,
        })
    }

    /// Convert a single field from JSON to DynamicValue.
    fn convert_field(
        &mut self,
        json: &JsonValue,
        move_type: &MoveType,
        field_name: &str,
    ) -> Result<DynamicValue> {
        match move_type {
            MoveType::Bool => {
                let v = json
                    .as_bool()
                    .ok_or_else(|| anyhow!("Expected bool for field {}", field_name))?;
                Ok(DynamicValue::Bool(v))
            }

            MoveType::U8 => {
                let v = parse_json_number_u64(json, field_name)? as u8;
                Ok(DynamicValue::U8(v))
            }

            MoveType::U16 => {
                let v = parse_json_number_u64(json, field_name)? as u16;
                Ok(DynamicValue::U16(v))
            }

            MoveType::U32 => {
                let v = parse_json_number_u64(json, field_name)? as u32;
                Ok(DynamicValue::U32(v))
            }

            MoveType::U64 => {
                let v = parse_json_number_u64(json, field_name)?;
                Ok(DynamicValue::U64(v))
            }

            MoveType::U128 => {
                let v = parse_json_number_u128(json, field_name)?;
                Ok(DynamicValue::U128(v))
            }

            MoveType::U256 => {
                let bytes = parse_json_u256(json, field_name)?;
                Ok(DynamicValue::U256(bytes))
            }

            MoveType::Address => {
                let addr_bytes = parse_json_address(json, field_name)?;
                Ok(DynamicValue::Address(addr_bytes))
            }

            MoveType::Signer => {
                let addr_bytes = parse_json_address(json, field_name)?;
                Ok(DynamicValue::Address(addr_bytes))
            }

            MoveType::Vector(inner_type) => self.convert_vector(json, inner_type, field_name),

            MoveType::Struct {
                address,
                module,
                name,
                type_args,
            } => self.convert_struct(json, address, module, name, type_args, field_name),

            MoveType::TypeParameter(_) => {
                Err(anyhow!("Unresolved type parameter in field {}", field_name))
            }
        }
    }

    /// Convert a vector field.
    fn convert_vector(
        &mut self,
        json: &JsonValue,
        inner_type: &MoveType,
        field_name: &str,
    ) -> Result<DynamicValue> {
        // Special case: vector<u8> might be stored as hex string or base64
        if matches!(inner_type, MoveType::U8) {
            if let Some(s) = json.as_str() {
                if let Some(hex_str) = s.strip_prefix("0x") {
                    let bytes = hex::decode(hex_str)
                        .with_context(|| format!("Invalid hex in field {}", field_name))?;
                    return Ok(DynamicValue::Vector(
                        bytes.into_iter().map(DynamicValue::U8).collect(),
                    ));
                }
                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(s) {
                    return Ok(DynamicValue::Vector(
                        bytes.into_iter().map(DynamicValue::U8).collect(),
                    ));
                }
            }
        }

        let arr = json
            .as_array()
            .ok_or_else(|| anyhow!("Expected array for field {}", field_name))?;

        let mut elements = Vec::new();
        for (i, elem) in arr.iter().enumerate() {
            let elem_name = format!("{}[{}]", field_name, i);
            let value = self.convert_field(elem, inner_type, &elem_name)?;
            elements.push(value);
        }

        Ok(DynamicValue::Vector(elements))
    }

    /// Convert a struct field.
    fn convert_struct(
        &mut self,
        json: &JsonValue,
        address: &AccountAddress,
        module: &str,
        name: &str,
        type_args: &[MoveType],
        field_name: &str,
    ) -> Result<DynamicValue> {
        let base_type = format!("{}::{}::{}", address.to_hex_literal(), module, name);

        let full_type = if type_args.is_empty() {
            base_type.clone()
        } else {
            let type_args_str = type_args
                .iter()
                .map(format_move_type)
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}<{}>", base_type, type_args_str)
        };

        // UID
        if base_type.contains("object::UID") || name == "UID" {
            return self.convert_uid(json, field_name);
        }

        // ID
        if base_type.contains("object::ID") || name == "ID" {
            return self.convert_id(json, field_name);
        }

        // Balance<T>
        if base_type.contains("balance::Balance") || name == "Balance" {
            return self.convert_balance(json, field_name);
        }

        // Option<T>
        if base_type.contains("option::Option") || name == "Option" {
            return self.convert_option(json, type_args, field_name);
        }

        // VecSet
        if name == "VecSet" {
            return self.convert_vec_set(json, type_args, field_name);
        }

        // VecMap
        if name == "VecMap" {
            return self.convert_vec_map(json, type_args, field_name);
        }

        // Table/Bag
        if name == "Table" || name == "Bag" || name == "ObjectTable" || name == "ObjectBag" {
            return self.convert_table_or_bag(json, field_name);
        }

        // String (0x1::string::String)
        if name == "String" && (module == "string" || module == "ascii") {
            return self.convert_string(json, field_name);
        }

        // TypeName
        if name == "TypeName" && module == "type_name" {
            return self.convert_type_name(json, field_name);
        }

        // dynamic_field::Field<K, V>
        // Field has specific layout: { id: UID, name: K, value: V }
        if name == "Field" && module == "dynamic_field" {
            return self.convert_dynamic_field(json, type_args, field_name);
        }

        // big_vector::Slice<E>
        // Slice has specific layout: { prev: u64, next: u64, keys: vector<u128>, vals: vector<E> }
        // TEMPORARILY DISABLED - testing layout_registry approach
        // if name == "Slice" && module == "big_vector" {
        //     return self.convert_big_vector_slice(json, type_args, field_name);
        // }

        // order::Order
        // TEMPORARILY DISABLED - testing layout_registry approach
        // if name == "Order" && module == "order" {
        //     return self.convert_order(json, field_name);
        // }

        // deep_price::OrderDeepPrice
        // TEMPORARILY DISABLED - testing layout_registry approach
        // if name == "OrderDeepPrice" && module == "deep_price" {
        //     return self.convert_order_deep_price(json, field_name);
        // }

        // Generic struct - try to get layout and recurse
        if let Some((layout, nested_type_args)) =
            self.layout_registry.get_layout_with_type_args(&full_type)
        {
            return self.json_to_dynamic_value_with_type_args(json, &layout, &nested_type_args);
        }

        // Fallback: try to process as generic object
        if let Some(obj) = json.as_object() {
            let mut fields = Vec::new();
            for (k, v) in obj {
                let value = self.infer_and_convert(v, k)?;
                fields.push((k.clone(), value));
            }
            return Ok(DynamicValue::Struct {
                type_name: name.to_string(),
                fields,
            });
        }

        Err(anyhow!(
            "Cannot convert struct {} for field {}",
            full_type,
            field_name
        ))
    }

    fn convert_uid(&mut self, json: &JsonValue, field_name: &str) -> Result<DynamicValue> {
        let id_obj = json
            .as_object()
            .ok_or_else(|| anyhow!("Expected object for UID in {}", field_name))?;

        let id_value = id_obj
            .get("id")
            .ok_or_else(|| anyhow!("Missing 'id' in UID for {}", field_name))?;

        let addr_bytes = parse_json_address(id_value, &format!("{}.id", field_name))?;

        Ok(DynamicValue::Struct {
            type_name: "UID".to_string(),
            fields: vec![(
                "id".to_string(),
                DynamicValue::Struct {
                    type_name: "ID".to_string(),
                    fields: vec![("bytes".to_string(), DynamicValue::Address(addr_bytes))],
                },
            )],
        })
    }

    fn convert_id(&mut self, json: &JsonValue, field_name: &str) -> Result<DynamicValue> {
        let addr_bytes = if let Some(obj) = json.as_object() {
            let id_value = obj
                .get("id")
                .ok_or_else(|| anyhow!("Missing 'id' in ID for {}", field_name))?;
            parse_json_address(id_value, &format!("{}.id", field_name))?
        } else {
            parse_json_address(json, field_name)?
        };

        Ok(DynamicValue::Struct {
            type_name: "ID".to_string(),
            fields: vec![("bytes".to_string(), DynamicValue::Address(addr_bytes))],
        })
    }

    fn convert_balance(&mut self, json: &JsonValue, field_name: &str) -> Result<DynamicValue> {
        let value = if let Some(obj) = json.as_object() {
            let v = obj
                .get("value")
                .ok_or_else(|| anyhow!("Missing 'value' in Balance for {}", field_name))?;
            parse_json_number_u64(v, &format!("{}.value", field_name))?
        } else {
            parse_json_number_u64(json, field_name)?
        };

        Ok(DynamicValue::Struct {
            type_name: "Balance".to_string(),
            fields: vec![("value".to_string(), DynamicValue::U64(value))],
        })
    }

    fn convert_option(
        &mut self,
        json: &JsonValue,
        type_args: &[MoveType],
        field_name: &str,
    ) -> Result<DynamicValue> {
        // Option is serialized as vector with 0 or 1 elements
        if json.is_null() {
            Ok(DynamicValue::Vector(vec![]))
        } else if let Some(inner_type) = type_args.first() {
            let inner = self.convert_field(json, inner_type, field_name)?;
            Ok(DynamicValue::Vector(vec![inner]))
        } else {
            let inner = self.infer_and_convert(json, field_name)?;
            Ok(DynamicValue::Vector(vec![inner]))
        }
    }

    fn convert_string(&mut self, json: &JsonValue, field_name: &str) -> Result<DynamicValue> {
        let s = json
            .as_str()
            .ok_or_else(|| anyhow!("Expected string for String in {}", field_name))?;

        let bytes: Vec<DynamicValue> = s.as_bytes().iter().map(|&b| DynamicValue::U8(b)).collect();
        Ok(DynamicValue::Struct {
            type_name: "String".to_string(),
            fields: vec![("bytes".to_string(), DynamicValue::Vector(bytes))],
        })
    }

    fn convert_type_name(&mut self, json: &JsonValue, field_name: &str) -> Result<DynamicValue> {
        let name_json = if let Some(obj) = json.as_object() {
            obj.get("name")
                .ok_or_else(|| anyhow!("Missing 'name' in TypeName for {}", field_name))?
        } else {
            json
        };

        let name_str = name_json
            .as_str()
            .ok_or_else(|| anyhow!("Expected string for TypeName.name in {}", field_name))?;

        let name_value = self.convert_string(
            &serde_json::Value::String(name_str.to_string()),
            &format!("{}.name", field_name),
        )?;

        Ok(DynamicValue::Struct {
            type_name: "TypeName".to_string(),
            fields: vec![("name".to_string(), name_value)],
        })
    }

    /// Convert dynamic_field::Field<K, V>
    ///
    /// Field struct has exactly three fields in this order: id, name, value
    /// - id: UID
    /// - name: K (the key type)
    /// - value: V (the value type)
    fn convert_dynamic_field(
        &mut self,
        json: &JsonValue,
        type_args: &[MoveType],
        field_name: &str,
    ) -> Result<DynamicValue> {
        let obj = json
            .as_object()
            .ok_or_else(|| anyhow!("Expected object for Field in {}", field_name))?;

        // Get the key type (K) and value type (V) from type args
        let key_type = type_args.first();
        let value_type = type_args.get(1);

        // 1. Convert id (UID)
        let id_json = obj
            .get("id")
            .ok_or_else(|| anyhow!("Missing 'id' in Field for {}", field_name))?;
        let id_value = self.convert_uid(id_json, &format!("{}.id", field_name))?;

        // 2. Convert name (K)
        let name_json = obj
            .get("name")
            .ok_or_else(|| anyhow!("Missing 'name' in Field for {}", field_name))?;
        let name_value = if let Some(key_t) = key_type {
            self.convert_field(name_json, key_t, &format!("{}.name", field_name))?
        } else {
            self.infer_and_convert(name_json, &format!("{}.name", field_name))?
        };

        // 3. Convert value (V)
        let value_json = obj
            .get("value")
            .ok_or_else(|| anyhow!("Missing 'value' in Field for {}", field_name))?;
        let value_value = if let Some(val_t) = value_type {
            self.convert_field(value_json, val_t, &format!("{}.value", field_name))?
        } else {
            self.infer_and_convert(value_json, &format!("{}.value", field_name))?
        };

        // Return with fields in the correct BCS order: id, name, value
        Ok(DynamicValue::Struct {
            type_name: "Field".to_string(),
            fields: vec![
                ("id".to_string(), id_value),
                ("name".to_string(), name_value),
                ("value".to_string(), value_value),
            ],
        })
    }

    fn convert_vec_set(
        &mut self,
        json: &JsonValue,
        type_args: &[MoveType],
        field_name: &str,
    ) -> Result<DynamicValue> {
        let obj = json
            .as_object()
            .ok_or_else(|| anyhow!("Expected object for VecSet in {}", field_name))?;

        let contents = obj
            .get("contents")
            .ok_or_else(|| anyhow!("Missing 'contents' in VecSet for {}", field_name))?;

        let arr = contents
            .as_array()
            .ok_or_else(|| anyhow!("Expected array in VecSet.contents for {}", field_name))?;

        let inner_type = type_args.first();
        let mut elements = Vec::new();
        for (i, elem) in arr.iter().enumerate() {
            let elem_name = format!("{}.contents[{}]", field_name, i);
            let value = if let Some(t) = inner_type {
                self.convert_field(elem, t, &elem_name)?
            } else {
                self.infer_and_convert(elem, &elem_name)?
            };
            elements.push(value);
        }

        Ok(DynamicValue::Struct {
            type_name: "VecSet".to_string(),
            fields: vec![("contents".to_string(), DynamicValue::Vector(elements))],
        })
    }

    fn convert_vec_map(
        &mut self,
        json: &JsonValue,
        type_args: &[MoveType],
        field_name: &str,
    ) -> Result<DynamicValue> {
        let obj = json
            .as_object()
            .ok_or_else(|| anyhow!("Expected object for VecMap in {}", field_name))?;

        let contents = obj
            .get("contents")
            .ok_or_else(|| anyhow!("Missing 'contents' in VecMap for {}", field_name))?;

        let arr = contents
            .as_array()
            .ok_or_else(|| anyhow!("Expected array in VecMap.contents for {}", field_name))?;

        let key_type = type_args.first();
        let val_type = type_args.get(1);

        let mut elements = Vec::new();
        for (i, entry) in arr.iter().enumerate() {
            let entry_obj = entry.as_object().ok_or_else(|| {
                anyhow!(
                    "Expected object in VecMap entry for {}.contents[{}]",
                    field_name,
                    i
                )
            })?;

            let key = entry_obj.get("key").ok_or_else(|| {
                anyhow!(
                    "Missing 'key' in VecMap entry {}.contents[{}]",
                    field_name,
                    i
                )
            })?;
            let value = entry_obj.get("value").ok_or_else(|| {
                anyhow!(
                    "Missing 'value' in VecMap entry {}.contents[{}]",
                    field_name,
                    i
                )
            })?;

            let key_val = if let Some(t) = key_type {
                self.convert_field(key, t, &format!("{}.contents[{}].key", field_name, i))?
            } else {
                self.infer_and_convert(key, &format!("{}.contents[{}].key", field_name, i))?
            };
            let val_val = if let Some(t) = val_type {
                self.convert_field(value, t, &format!("{}.contents[{}].value", field_name, i))?
            } else {
                self.infer_and_convert(value, &format!("{}.contents[{}].value", field_name, i))?
            };

            elements.push(DynamicValue::Struct {
                type_name: "Entry".to_string(),
                fields: vec![("key".to_string(), key_val), ("value".to_string(), val_val)],
            });
        }

        Ok(DynamicValue::Struct {
            type_name: "VecMap".to_string(),
            fields: vec![("contents".to_string(), DynamicValue::Vector(elements))],
        })
    }

    fn convert_table_or_bag(&mut self, json: &JsonValue, field_name: &str) -> Result<DynamicValue> {
        let obj = json
            .as_object()
            .ok_or_else(|| anyhow!("Expected object for Table/Bag in {}", field_name))?;

        let id_json = obj
            .get("id")
            .ok_or_else(|| anyhow!("Missing 'id' in Table/Bag for {}", field_name))?;
        let id_value = self.convert_uid(id_json, &format!("{}.id", field_name))?;

        let size_json = obj
            .get("size")
            .ok_or_else(|| anyhow!("Missing 'size' in Table/Bag for {}", field_name))?;
        let size = parse_json_number_u64(size_json, &format!("{}.size", field_name))?;

        Ok(DynamicValue::Struct {
            type_name: "Table".to_string(),
            fields: vec![
                ("id".to_string(), id_value),
                ("size".to_string(), DynamicValue::U64(size)),
            ],
        })
    }

    /// Infer type from JSON value and convert.
    fn infer_and_convert(&mut self, json: &JsonValue, field_name: &str) -> Result<DynamicValue> {
        match json {
            JsonValue::Null => Ok(DynamicValue::Vector(vec![])),
            JsonValue::Bool(b) => Ok(DynamicValue::Bool(*b)),
            JsonValue::Number(n) => {
                if let Some(v) = n.as_u64() {
                    Ok(DynamicValue::U64(v))
                } else if let Some(v) = n.as_i64() {
                    Ok(DynamicValue::U64(v as u64))
                } else {
                    Err(anyhow!("Cannot convert number for {}", field_name))
                }
            }
            JsonValue::String(s) => {
                if s.starts_with("0x") && s.len() == 66 {
                    let bytes = parse_hex_address(s)?;
                    Ok(DynamicValue::Address(bytes))
                } else if s.chars().all(|c| c.is_ascii_digit()) {
                    let v: u64 = s.parse().with_context(|| {
                        format!("Failed to parse numeric string for {}", field_name)
                    })?;
                    Ok(DynamicValue::U64(v))
                } else {
                    Ok(DynamicValue::Vector(
                        s.as_bytes().iter().map(|b| DynamicValue::U8(*b)).collect(),
                    ))
                }
            }
            JsonValue::Array(arr) => {
                let mut elements = Vec::new();
                for (i, elem) in arr.iter().enumerate() {
                    let value = self.infer_and_convert(elem, &format!("{}[{}]", field_name, i))?;
                    elements.push(value);
                }
                Ok(DynamicValue::Vector(elements))
            }
            JsonValue::Object(_) => {
                if let Some(obj) = json.as_object() {
                    // UID/ID pattern
                    if obj.contains_key("id") && obj.len() == 1 {
                        if let Some(id_val) = obj.get("id") {
                            if id_val.is_string() || id_val.is_object() {
                                return self.convert_uid(json, field_name);
                            }
                        }
                    }

                    // Balance pattern
                    if obj.contains_key("value") && obj.len() == 1 {
                        return self.convert_balance(json, field_name);
                    }

                    // VecSet/VecMap pattern
                    if obj.contains_key("contents") {
                        if let Some(contents) = obj.get("contents") {
                            if contents.is_array() {
                                if let Some(first) = contents.as_array().and_then(|a| a.first()) {
                                    if first.is_object()
                                        && first.as_object().is_some_and(|o| o.contains_key("key"))
                                    {
                                        return self.convert_vec_map(json, &[], field_name);
                                    }
                                }
                                return self.convert_vec_set(json, &[], field_name);
                            }
                        }
                    }

                    // Table/Bag pattern
                    if obj.contains_key("id") && obj.contains_key("size") {
                        return self.convert_table_or_bag(json, field_name);
                    }
                }

                let obj = json.as_object().unwrap();
                let mut fields = Vec::new();
                for (k, v) in obj {
                    let value = self.infer_and_convert(v, &format!("{}.{}", field_name, k))?;
                    fields.push((k.clone(), value));
                }
                Ok(DynamicValue::Struct {
                    type_name: "Unknown".to_string(),
                    fields,
                })
            }
        }
    }
}

impl Default for JsonToBcsConverter {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

fn parse_json_number_u64(json: &JsonValue, field_name: &str) -> Result<u64> {
    if let Some(n) = json.as_u64() {
        return Ok(n);
    }
    if let Some(n) = json.as_i64() {
        return Ok(n as u64);
    }
    if let Some(s) = json.as_str() {
        return s
            .parse()
            .with_context(|| format!("Failed to parse '{}' as u64 for {}", s, field_name));
    }
    Err(anyhow!(
        "Expected number or numeric string for {}, got {:?}",
        field_name,
        json
    ))
}

fn parse_json_number_u128(json: &JsonValue, field_name: &str) -> Result<u128> {
    if let Some(s) = json.as_str() {
        return s
            .parse()
            .with_context(|| format!("Failed to parse '{}' as u128 for {}", s, field_name));
    }
    if let Some(n) = json.as_u64() {
        return Ok(n as u128);
    }
    Err(anyhow!(
        "Expected numeric string for u128 field {}",
        field_name
    ))
}

fn parse_json_u256(json: &JsonValue, field_name: &str) -> Result<[u8; 32]> {
    if let Some(s) = json.as_str() {
        if let Some(hex_str) = s.strip_prefix("0x") {
            let bytes = hex::decode(hex_str)
                .with_context(|| format!("Invalid hex for U256 {}", field_name))?;
            if bytes.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                return Ok(arr);
            }
        }
        let n: u128 = s
            .parse()
            .with_context(|| format!("Failed to parse U256 for {}", field_name))?;
        let mut arr = [0u8; 32];
        arr[16..].copy_from_slice(&n.to_le_bytes());
        return Ok(arr);
    }
    Err(anyhow!("Expected string for U256 field {}", field_name))
}

fn parse_json_address(json: &JsonValue, field_name: &str) -> Result<[u8; 32]> {
    if let Some(s) = json.as_str() {
        return parse_hex_address(s).with_context(|| format!("Invalid address for {}", field_name));
    }
    if let Some(obj) = json.as_object() {
        if let Some(id) = obj.get("id") {
            return parse_json_address(id, field_name);
        }
    }
    Err(anyhow!(
        "Expected hex string for address field {}",
        field_name
    ))
}

fn parse_hex_address(s: &str) -> Result<[u8; 32]> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    let padded = format!("{:0>64}", s);
    let bytes = hex::decode(&padded).with_context(|| format!("Invalid hex address: 0x{}", s))?;

    if bytes.len() != 32 {
        return Err(anyhow!("Address must be 32 bytes, got {}", bytes.len()));
    }

    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

fn format_move_type(move_type: &MoveType) -> String {
    match move_type {
        MoveType::Bool => "bool".to_string(),
        MoveType::U8 => "u8".to_string(),
        MoveType::U16 => "u16".to_string(),
        MoveType::U32 => "u32".to_string(),
        MoveType::U64 => "u64".to_string(),
        MoveType::U128 => "u128".to_string(),
        MoveType::U256 => "u256".to_string(),
        MoveType::Address => "address".to_string(),
        MoveType::Signer => "signer".to_string(),
        MoveType::Vector(inner) => format!("vector<{}>", format_move_type(inner)),
        MoveType::Struct {
            address,
            module,
            name,
            type_args,
        } => {
            let base = format!("{}::{}::{}", address.to_hex_literal(), module, name);
            if type_args.is_empty() {
                base
            } else {
                let args_str = type_args
                    .iter()
                    .map(format_move_type)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}<{}>", base, args_str)
            }
        }
        MoveType::TypeParameter(idx) => format!("T{}", idx),
    }
}
