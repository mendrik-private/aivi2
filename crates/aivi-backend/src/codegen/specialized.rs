impl<'a> CraneliftCompiler<'a, ObjectModule> {
    fn new(program: &'a Program) -> Result<Self, CodegenError> {
        let isa = build_target_isa()?;
        let module = ObjectModule::new(
            ObjectBuilder::new(isa, "aivi_backend", default_libcall_names()).map_err(|error| {
                CodegenError::ObjectModuleCreation {
                    message: error.to_string().into_boxed_str(),
                }
            })?,
        );
        Ok(CraneliftCompiler::with_module(program, module, None))
    }
}

impl<'a> CraneliftCompiler<'a, JITModule> {
    fn new_jit(program: &'a Program) -> Result<Self, CodegenError> {
        let isa = build_target_isa()?;
        let jit_symbols = Arc::new(Mutex::new(BTreeMap::new()));
        let lookup_symbols = Arc::clone(&jit_symbols);
        let mut builder = JITBuilder::with_isa(isa, default_libcall_names());
        builder.symbol_lookup_fn(Box::new(move |symbol| {
            lookup_symbols
                .lock()
                .ok()
                .and_then(|symbols| symbols.get(symbol).copied())
                .map(|address| address as *const u8)
                .or_else(|| aivi_ffi_call::lookup_runtime_symbol(symbol))
        }));
        let module = JITModule::new(builder);
        Ok(CraneliftCompiler::with_module(
            program,
            module,
            Some(jit_symbols),
        ))
    }
}

impl<'a> CraneliftCompiler<'a, ObjectModule> {
    /// Three-phase compilation:
    /// 1. Sequential declaration — declares all kernel function symbols in the module.
    /// 2. Sequential CLIF build — lowers each kernel IR to Cranelift CLIF IR.
    /// 3. **Parallel Cranelift compile** — `ctx.compile(isa)` is data-parallel; each
    ///    kernel produces independent machine code with no module mutations.
    /// 4. Sequential emit — writes the compiled machine code bytes into the object module.
    ///
    /// Phases 1–2 and 4 remain sequential because they mutate shared module state.
    /// Phase 3 uses Rayon's par_iter and yields a speedup proportional to kernel count.
    fn compile(mut self) -> Result<CompiledProgram, CodegenErrors> {
        let kernel_ids = self.non_ambient_kernel_ids();
        self.prevalidate_kernels(kernel_ids.iter().copied())?;
        self.declare_kernels(kernel_ids.iter().copied(), KernelLinkage::Local)?;
        let built_kernels = self.build_kernels(kernel_ids.iter().copied())?;
        self.finish_object_compilation(built_kernels)
    }

    fn compile_kernel(
        mut self,
        kernel_id: KernelId,
    ) -> Result<CompiledKernelArtifact, CodegenErrors> {
        self.ensure_compileable_kernel(kernel_id)
            .map_err(wrap_one)?;
        self.prevalidate_kernels([kernel_id])?;
        self.declare_kernels([kernel_id], KernelLinkage::Local)?;
        let built_kernel = self
            .build_kernels([kernel_id])?
            .into_iter()
            .next()
            .expect("single-kernel build should yield exactly one CLIF artifact");
        let compiled = self.finish_object_compilation(vec![built_kernel])?;
        compiled.into_single_kernel_artifact().ok_or_else(|| {
            wrap_one(CodegenError::CraneliftModule {
                kernel: Some(kernel_id),
                message: "single-kernel compilation should emit exactly one kernel artifact".into(),
            })
        })
    }

    fn finish_object_compilation(
        mut self,
        built_kernels: Vec<BuiltKernel>,
    ) -> Result<CompiledProgram, CodegenErrors> {
        let emit_inputs = self.compile_machine_code(built_kernels)?;
        let alignment = self.module.isa().function_alignment().minimum as u64;
        let mut compiled_kernels = Vec::with_capacity(emit_inputs.len());
        let mut emit_errors = Vec::new();
        for (built, code_size) in emit_inputs {
            let compiled = built
                .ctx
                .compiled_code()
                .expect("compilation succeeded in phase 3");
            let bytes = compiled.code_buffer();
            let relocs: Vec<ModuleReloc> = compiled
                .buffer
                .relocs()
                .iter()
                .map(|reloc| ModuleReloc::from_mach_reloc(reloc, &built.ctx.func, built.func_id))
                .collect();
            if let Err(error) =
                self.module
                    .define_function_bytes(built.func_id, alignment, bytes, &relocs)
            {
                emit_errors.push(CodegenError::CraneliftModule {
                    kernel: Some(built.kernel_id),
                    message: error.to_string().into_boxed_str(),
                });
            } else {
                compiled_kernels.push(self.compiled_kernel_metadata(
                    built.kernel_id,
                    built.symbol,
                    built.clif,
                    code_size,
                ));
            }
        }
        if !emit_errors.is_empty() {
            return Err(CodegenErrors::new(emit_errors));
        }

        let object = self.module.finish().emit().map_err(|error| {
            wrap_one(CodegenError::ObjectEmission {
                message: error.to_string().into_boxed_str(),
            })
        })?;
        let kernel_index = compiled_kernels
            .iter()
            .enumerate()
            .map(|(index, kernel)| (kernel.kernel, index))
            .collect();

        Ok(CompiledProgram {
            object,
            kernels: compiled_kernels,
            kernel_index,
        })
    }
}

impl<'a> CraneliftCompiler<'a, JITModule> {
    fn compile_kernel_jit_with_cache_artifact(
        mut self,
        kernel_id: KernelId,
    ) -> Result<(CompiledJitKernel, Option<CachedJitKernelArtifact>), CodegenErrors> {
        let kernel_ids = jit_dependency_kernel_ids(self.program, kernel_id).map_err(wrap_one)?;
        self.prevalidate_kernels(kernel_ids.iter().copied())?;
        self.declare_kernels(kernel_ids.iter().copied(), KernelLinkage::Local)?;
        let built_kernels = self.build_kernels(kernel_ids.iter().copied())?;
        self.finish_jit_compilation(kernel_id, built_kernels)
    }

    fn finish_jit_compilation(
        mut self,
        requested_kernel: KernelId,
        built_kernels: Vec<BuiltKernel>,
    ) -> Result<(CompiledJitKernel, Option<CachedJitKernelArtifact>), CodegenErrors> {
        let emit_inputs = self.compile_machine_code(built_kernels)?;
        let alignment = self.module.isa().function_alignment().minimum as u64;
        let mut compiled_metadata = BTreeMap::new();
        let mut requested_func_id = None;
        let mut cached_kernels = Vec::with_capacity(emit_inputs.len());
        let mut cacheable = true;
        let mut emit_errors = Vec::new();
        for (built, code_size) in emit_inputs {
            let compiled = built
                .ctx
                .compiled_code()
                .expect("compilation succeeded in phase 3");
            let bytes = compiled.code_buffer().to_vec().into_boxed_slice();
            let relocs: Vec<ModuleReloc> = compiled
                .buffer
                .relocs()
                .iter()
                .map(|reloc| ModuleReloc::from_mach_reloc(reloc, &built.ctx.func, built.func_id))
                .collect();
            if let Err(error) =
                self.module
                    .define_function_bytes(built.func_id, alignment, bytes.as_ref(), &relocs)
            {
                emit_errors.push(CodegenError::CraneliftModule {
                    kernel: Some(built.kernel_id),
                    message: error.to_string().into_boxed_str(),
                });
                continue;
            }
            if cacheable {
                match self.cacheable_jit_kernel(built.kernel_id, bytes.clone(), &relocs) {
                    Some(cached) => cached_kernels.push(cached),
                    None => cacheable = false,
                }
            }
            if built.kernel_id == requested_kernel {
                requested_func_id = Some(built.func_id);
            }
            compiled_metadata.insert(
                built.kernel_id,
                self.compiled_kernel_metadata(built.kernel_id, built.symbol, built.clif, code_size),
            );
        }
        if !emit_errors.is_empty() {
            return Err(CodegenErrors::new(emit_errors));
        }

        self.ensure_supported_jit_externals(requested_kernel)
            .map_err(wrap_one)?;
        let requested_func_id = requested_func_id.ok_or_else(|| {
            wrap_one(CodegenError::MissingKernel {
                kernel: requested_kernel,
            })
        })?;
        let (signal_slots, imported_item_slots) =
            self.materialize_jit_data_slots(requested_kernel)?;
        self.module.finalize_definitions().map_err(|error| {
            wrap_one(CodegenError::CraneliftModule {
                kernel: Some(requested_kernel),
                message: error.to_string().into_boxed_str(),
            })
        })?;
        let function = self.module.get_finalized_function(requested_func_id);
        let caller = FunctionCaller::new(
            self.build_jit_call_signature(requested_kernel)
                .map_err(wrap_one)?,
        );
        compiled_metadata.remove(&requested_kernel).ok_or_else(|| {
            wrap_one(CodegenError::MissingKernel {
                kernel: requested_kernel,
            })
        })?;
        let cached_artifact =
            cacheable.then(|| self.build_cached_jit_artifact(requested_kernel, cached_kernels));

        Ok((
            CompiledJitKernel {
                function,
                caller,
                signal_slots,
                imported_item_slots,
                _module: self.module,
            },
            cached_artifact,
        ))
    }

    fn replay_cached_jit_kernel(
        mut self,
        requested_kernel: KernelId,
        artifact: &CachedJitKernelArtifact,
    ) -> Result<CompiledJitKernel, CodegenErrors> {
        let kernel_ids =
            jit_dependency_kernel_ids(self.program, requested_kernel).map_err(wrap_one)?;
        if artifact.requested_kernel != requested_kernel
            || artifact
                .kernels
                .iter()
                .map(|kernel| kernel.kernel)
                .collect::<Vec<_>>()
                != kernel_ids
        {
            return Err(wrap_one(CodegenError::CraneliftModule {
                kernel: Some(requested_kernel),
                message:
                    "cached JIT artifact does not match the requested kernel dependency closure"
                        .into(),
            }));
        }

        self.prevalidate_kernels(kernel_ids.iter().copied())?;
        self.declare_kernels(kernel_ids.iter().copied(), KernelLinkage::Local)?;
        for slot in &artifact.signal_slots {
            self.declare_signal_item_slot(slot.item, slot.layout)
                .map_err(wrap_one)?;
        }
        for slot in &artifact.imported_item_slots {
            self.declare_imported_item_slot(slot.item, slot.layout)
                .map_err(wrap_one)?;
        }
        for descriptor in &artifact.callable_descriptors {
            self.declare_callable_item_descriptor(
                descriptor.item,
                descriptor.body,
                descriptor.arity as usize,
            )
            .map_err(wrap_one)?;
        }
        for symbol in &artifact.external_funcs {
            self.ensure_named_external_func_declared(symbol)
                .map_err(wrap_one)?;
        }
        for literal in &artifact.literal_data {
            self.define_cached_literal_data(&literal.symbol, literal.bytes.clone(), literal.align)
                .map_err(wrap_one)?;
        }

        let alignment = self.module.isa().function_alignment().minimum as u64;
        let mut requested_func_id = None;
        for kernel in &artifact.kernels {
            let func_id = self
                .declared_functions
                .get(&kernel.kernel)
                .copied()
                .expect("replayed JIT kernels were declared before machine code definition");
            let relocs = kernel
                .relocs
                .iter()
                .map(|reloc| self.materialize_cached_jit_reloc(reloc))
                .collect::<Result<Vec<_>, _>>()
                .map_err(wrap_one)?;
            self.module
                .define_function_bytes(func_id, alignment, kernel.bytes.as_ref(), &relocs)
                .map_err(|error| {
                    wrap_one(CodegenError::CraneliftModule {
                        kernel: Some(kernel.kernel),
                        message: error.to_string().into_boxed_str(),
                    })
                })?;
            if kernel.kernel == requested_kernel {
                requested_func_id = Some(func_id);
            }
        }

        self.ensure_supported_jit_externals(requested_kernel)
            .map_err(wrap_one)?;
        let requested_func_id = requested_func_id.ok_or_else(|| {
            wrap_one(CodegenError::MissingKernel {
                kernel: requested_kernel,
            })
        })?;
        let (signal_slots, imported_item_slots) =
            self.materialize_jit_data_slots(requested_kernel)?;
        self.module.finalize_definitions().map_err(|error| {
            wrap_one(CodegenError::CraneliftModule {
                kernel: Some(requested_kernel),
                message: error.to_string().into_boxed_str(),
            })
        })?;
        let function = self.module.get_finalized_function(requested_func_id);
        let caller = FunctionCaller::new(
            self.build_jit_call_signature(requested_kernel)
                .map_err(wrap_one)?,
        );

        Ok(CompiledJitKernel {
            function,
            caller,
            signal_slots,
            imported_item_slots,
            _module: self.module,
        })
    }

    fn build_cached_jit_artifact(
        &self,
        requested_kernel: KernelId,
        kernels: Vec<CachedJitCompiledKernel>,
    ) -> CachedJitKernelArtifact {
        CachedJitKernelArtifact {
            requested_kernel,
            kernels,
            signal_slots: self
                .signal_slot_layouts
                .iter()
                .map(|(&item, &layout)| CachedJitDataSlot { item, layout })
                .collect(),
            imported_item_slots: self
                .imported_item_slot_layouts
                .iter()
                .map(|(&item, &layout)| CachedJitDataSlot { item, layout })
                .collect(),
            callable_descriptors: self
                .callable_descriptor_specs
                .iter()
                .map(|(&item, &(body, arity))| CachedJitCallableDescriptor {
                    item,
                    body,
                    arity: arity as u32,
                })
                .collect(),
            literal_data: self
                .literal_data
                .iter()
                .map(|(symbol, record)| CachedJitLiteralData {
                    symbol: symbol.clone(),
                    align: record.align,
                    bytes: record.bytes.clone(),
                })
                .collect(),
            external_funcs: self.declared_external_funcs.keys().cloned().collect(),
        }
    }

    fn cacheable_jit_kernel(
        &self,
        kernel: KernelId,
        bytes: Box<[u8]>,
        relocs: &[ModuleReloc],
    ) -> Option<CachedJitCompiledKernel> {
        let relocs = relocs
            .iter()
            .map(|reloc| self.cacheable_jit_reloc(reloc))
            .collect::<Option<Vec<_>>>()?;
        Some(CachedJitCompiledKernel {
            kernel,
            bytes,
            relocs,
        })
    }

    fn cacheable_jit_reloc(&self, reloc: &ModuleReloc) -> Option<CachedJitReloc> {
        Some(CachedJitReloc {
            offset: reloc.offset,
            kind: reloc.kind,
            target: self.cacheable_jit_reloc_target(&reloc.name)?,
            addend: reloc.addend,
        })
    }

    fn cacheable_jit_reloc_target(
        &self,
        target: &ModuleRelocTarget,
    ) -> Option<CachedJitRelocTarget> {
        match target {
            ModuleRelocTarget::User {
                namespace: 0,
                index,
            } => Some(CachedJitRelocTarget::Function(
                self.cacheable_jit_function_target(FuncId::from_u32(*index))?,
            )),
            ModuleRelocTarget::User {
                namespace: 1,
                index,
            } => Some(CachedJitRelocTarget::Data(
                self.cacheable_jit_data_target(DataId::from_u32(*index))?,
            )),
            ModuleRelocTarget::FunctionOffset(func_id, offset) => {
                Some(CachedJitRelocTarget::FunctionOffset {
                    target: self.cacheable_jit_function_target(*func_id)?,
                    offset: *offset,
                })
            }
            ModuleRelocTarget::LibCall(libcall) => {
                Some(CachedJitRelocTarget::LibCall(libcall.to_string().into()))
            }
            ModuleRelocTarget::KnownSymbol(symbol) => {
                Some(CachedJitRelocTarget::KnownSymbol(symbol.to_string().into()))
            }
            ModuleRelocTarget::User { .. } => None,
        }
    }

    fn cacheable_jit_function_target(&self, func_id: FuncId) -> Option<CachedJitFunctionTarget> {
        self.declared_functions
            .iter()
            .find_map(|(&kernel, &declared)| {
                (declared == func_id).then_some(CachedJitFunctionTarget::Kernel(kernel))
            })
            .or_else(|| {
                self.declared_external_funcs
                    .iter()
                    .find_map(|(symbol, &declared)| {
                        (declared == func_id)
                            .then_some(CachedJitFunctionTarget::External(symbol.clone()))
                    })
            })
    }

    fn cacheable_jit_data_target(&self, data_id: DataId) -> Option<CachedJitDataTarget> {
        self.declared_signal_slots
            .iter()
            .find_map(|(&item, &declared)| {
                (declared == data_id).then_some(CachedJitDataTarget::SignalSlot(item))
            })
            .or_else(|| {
                self.declared_imported_item_slots
                    .iter()
                    .find_map(|(&item, &declared)| {
                        (declared == data_id).then_some(CachedJitDataTarget::ImportedItemSlot(item))
                    })
            })
            .or_else(|| {
                self.declared_callable_descriptors
                    .iter()
                    .find_map(|(&item, &declared)| {
                        (declared == data_id)
                            .then_some(CachedJitDataTarget::CallableDescriptor(item))
                    })
            })
            .or_else(|| {
                self.literal_data.iter().find_map(|(symbol, record)| {
                    (record.data_id == data_id)
                        .then_some(CachedJitDataTarget::Literal(symbol.clone()))
                })
            })
    }

    fn materialize_cached_jit_reloc(
        &self,
        reloc: &CachedJitReloc,
    ) -> Result<ModuleReloc, CodegenError> {
        Ok(ModuleReloc {
            offset: reloc.offset,
            kind: reloc.kind,
            name: self.materialize_cached_jit_reloc_target(&reloc.target)?,
            addend: reloc.addend,
        })
    }

    fn materialize_cached_jit_reloc_target(
        &self,
        target: &CachedJitRelocTarget,
    ) -> Result<ModuleRelocTarget, CodegenError> {
        match target {
            CachedJitRelocTarget::Function(target) => {
                Ok(self.resolve_cached_jit_function_target(target)?.into())
            }
            CachedJitRelocTarget::FunctionOffset { target, offset } => {
                Ok(ModuleRelocTarget::FunctionOffset(
                    self.resolve_cached_jit_function_target(target)?,
                    *offset,
                ))
            }
            CachedJitRelocTarget::Data(target) => {
                Ok(self.resolve_cached_jit_data_target(target)?.into())
            }
            CachedJitRelocTarget::LibCall(symbol) => symbol
                .parse::<cranelift_codegen::ir::LibCall>()
                .map(ModuleRelocTarget::LibCall)
                .map_err(|_| CodegenError::CraneliftModule {
                    kernel: None,
                    message: format!("unknown cached libcall relocation target `{symbol}`").into(),
                }),
            CachedJitRelocTarget::KnownSymbol(symbol) => symbol
                .parse::<cranelift_codegen::ir::KnownSymbol>()
                .map(ModuleRelocTarget::KnownSymbol)
                .map_err(|_| CodegenError::CraneliftModule {
                    kernel: None,
                    message: format!("unknown cached known-symbol relocation target `{symbol}`")
                        .into(),
                }),
        }
    }

    fn resolve_cached_jit_function_target(
        &self,
        target: &CachedJitFunctionTarget,
    ) -> Result<FuncId, CodegenError> {
        match target {
            CachedJitFunctionTarget::Kernel(kernel) => self
                .declared_functions
                .get(kernel)
                .copied()
                .ok_or_else(|| CodegenError::MissingKernel { kernel: *kernel }),
            CachedJitFunctionTarget::External(symbol) => self
                .declared_external_funcs
                .get(symbol)
                .copied()
                .ok_or_else(|| CodegenError::CraneliftModule {
                    kernel: None,
                    message: format!(
                        "cached JIT artifact references unknown external symbol `{symbol}`"
                    )
                    .into(),
                }),
        }
    }

    fn resolve_cached_jit_data_target(
        &self,
        target: &CachedJitDataTarget,
    ) -> Result<DataId, CodegenError> {
        match target {
            CachedJitDataTarget::SignalSlot(item) => self
                .declared_signal_slots
                .get(item)
                .copied()
                .ok_or_else(|| CodegenError::CraneliftModule {
                    kernel: None,
                    message: format!("missing cached JIT signal slot for item{item}").into(),
                }),
            CachedJitDataTarget::ImportedItemSlot(item) => self
                .declared_imported_item_slots
                .get(item)
                .copied()
                .ok_or_else(|| CodegenError::CraneliftModule {
                    kernel: None,
                    message: format!("missing cached JIT imported-item slot for item{item}").into(),
                }),
            CachedJitDataTarget::CallableDescriptor(item) => self
                .declared_callable_descriptors
                .get(item)
                .copied()
                .ok_or_else(|| CodegenError::CraneliftModule {
                    kernel: None,
                    message: format!("missing cached JIT callable descriptor for item{item}")
                        .into(),
                }),
            CachedJitDataTarget::Literal(symbol) => self
                .literal_data
                .get(symbol)
                .map(|record| record.data_id)
                .ok_or_else(|| CodegenError::CraneliftModule {
                    kernel: None,
                    message: format!("missing cached JIT literal data `{symbol}`").into(),
                }),
        }
    }

    fn ensure_named_external_func_declared(
        &mut self,
        symbol: &str,
    ) -> Result<FuncId, CodegenError> {
        if let Some(&func_id) = self.declared_external_funcs.get(symbol) {
            return Ok(func_id);
        }
        let mut sig = self.module.make_signature();
        match symbol {
            "aivi_text_concat" => {
                sig.params.push(AbiParam::new(types::I64));
                sig.params.push(AbiParam::new(self.pointer_type()));
                sig.returns.push(AbiParam::new(self.pointer_type()));
            }
            "aivi_bytes_append" => {
                sig.params.push(AbiParam::new(self.pointer_type()));
                sig.params.push(AbiParam::new(self.pointer_type()));
                sig.returns.push(AbiParam::new(self.pointer_type()));
            }
            "aivi_bytes_repeat" => {
                sig.params.push(AbiParam::new(types::I64));
                sig.params.push(AbiParam::new(types::I64));
                sig.returns.push(AbiParam::new(self.pointer_type()));
            }
            "aivi_bytes_slice" => {
                sig.params.push(AbiParam::new(types::I64));
                sig.params.push(AbiParam::new(types::I64));
                sig.params.push(AbiParam::new(self.pointer_type()));
                sig.returns.push(AbiParam::new(self.pointer_type()));
            }
            "aivi_arena_alloc" => {
                sig.params.push(AbiParam::new(types::I64));
                sig.params.push(AbiParam::new(types::I64));
                sig.returns.push(AbiParam::new(self.pointer_type()));
            }
            "aivi_list_new" | "aivi_set_new" => {
                sig.params.push(AbiParam::new(types::I64));
                sig.params.push(AbiParam::new(self.pointer_type()));
                sig.params.push(AbiParam::new(types::I64));
                sig.returns.push(AbiParam::new(self.pointer_type()));
            }
            "aivi_map_new" => {
                sig.params.push(AbiParam::new(types::I64));
                sig.params.push(AbiParam::new(self.pointer_type()));
                sig.params.push(AbiParam::new(types::I64));
                sig.params.push(AbiParam::new(types::I64));
                sig.returns.push(AbiParam::new(self.pointer_type()));
            }
            "aivi_list_len" => {
                sig.params.push(AbiParam::new(self.pointer_type()));
                sig.returns.push(AbiParam::new(types::I64));
            }
            "aivi_list_get" => {
                sig.params.push(AbiParam::new(self.pointer_type()));
                sig.params.push(AbiParam::new(types::I64));
                sig.returns.push(AbiParam::new(self.pointer_type()));
            }
            "aivi_list_slice" => {
                sig.params.push(AbiParam::new(self.pointer_type()));
                sig.params.push(AbiParam::new(types::I64));
                sig.params.push(AbiParam::new(types::I64));
                sig.returns.push(AbiParam::new(self.pointer_type()));
            }
            "aivi_decimal_add" | "aivi_decimal_sub" | "aivi_decimal_mul" | "aivi_decimal_div"
            | "aivi_decimal_mod" | "aivi_bigint_add" | "aivi_bigint_sub" | "aivi_bigint_mul"
            | "aivi_bigint_div" | "aivi_bigint_mod" => {
                sig.params.push(AbiParam::new(self.pointer_type()));
                sig.params.push(AbiParam::new(self.pointer_type()));
                sig.returns.push(AbiParam::new(self.pointer_type()));
            }
            "aivi_decimal_eq" | "aivi_decimal_gt" | "aivi_decimal_lt" | "aivi_decimal_gte"
            | "aivi_decimal_lte" | "aivi_bigint_eq" | "aivi_bigint_gt" | "aivi_bigint_lt"
            | "aivi_bigint_gte" | "aivi_bigint_lte" => {
                sig.params.push(AbiParam::new(self.pointer_type()));
                sig.params.push(AbiParam::new(self.pointer_type()));
                sig.returns.push(AbiParam::new(types::I8));
            }
            _ => {
                return Err(CodegenError::CraneliftModule {
                    kernel: None,
                    message: format!(
                        "cached JIT artifact references external symbol `{symbol}` without a known lazy-JIT signature"
                    )
                    .into(),
                });
            }
        }
        let func_id = self
            .module
            .declare_function(symbol, Linkage::Import, &sig)
            .map_err(|error| CodegenError::CraneliftModule {
                kernel: None,
                message: error.to_string().into_boxed_str(),
            })?;
        self.declared_external_funcs
            .insert(symbol.to_owned().into_boxed_str(), func_id);
        Ok(func_id)
    }

    fn define_cached_literal_data(
        &mut self,
        symbol: &str,
        bytes: Box<[u8]>,
        align: u64,
    ) -> Result<DataId, CodegenError> {
        if let Some(record) = self.literal_data.get(symbol) {
            return Ok(record.data_id);
        }
        let data_id = self
            .module
            .declare_data(symbol, Linkage::Local, false, false)
            .map_err(|error| CodegenError::CraneliftModule {
                kernel: None,
                message: error.to_string().into_boxed_str(),
            })?;
        let stored_bytes = bytes.clone();
        let mut data = DataDescription::new();
        data.define(bytes);
        data.set_align(align);
        self.module
            .define_data(data_id, &data)
            .map_err(|error| CodegenError::CraneliftModule {
                kernel: None,
                message: error.to_string().into_boxed_str(),
            })?;
        self.literal_data.insert(
            symbol.to_owned().into_boxed_str(),
            JitLiteralDataRecord {
                data_id,
                align,
                bytes: stored_bytes,
            },
        );
        Ok(data_id)
    }

    fn ensure_supported_jit_externals(&self, kernel_id: KernelId) -> Result<(), CodegenError> {
        for symbol in self.declared_external_funcs.keys() {
            if aivi_ffi_call::lookup_runtime_symbol(symbol).is_none() {
                return Err(CodegenError::UnsupportedJitSymbol {
                    kernel: kernel_id,
                    symbol: symbol.clone(),
                });
            }
        }
        Ok(())
    }

    fn materialize_jit_data_slots(
        &mut self,
        kernel_id: KernelId,
    ) -> Result<(Vec<JitDataSlot>, Vec<JitDataSlot>), CodegenErrors> {
        let Some(symbols) = self.jit_symbols.as_ref() else {
            return Err(wrap_one(CodegenError::JitModuleCreation {
                message: "JIT compiler was created without a symbol table".into(),
            }));
        };
        let mut symbol_table = symbols.lock().map_err(|_| {
            wrap_one(CodegenError::JitModuleCreation {
                message: "JIT symbol table is poisoned".into(),
            })
        })?;
        let signal_slots = self.build_jit_slots(
            kernel_id,
            &self.signal_slot_layouts,
            |program, item| signal_slot_symbol(program, item),
            &mut symbol_table,
        )?;
        let imported_item_slots = self.build_jit_slots(
            kernel_id,
            &self.imported_item_slot_layouts,
            |program, item| imported_item_slot_symbol(program, item),
            &mut symbol_table,
        )?;
        Ok((signal_slots, imported_item_slots))
    }

    fn build_jit_slots(
        &self,
        kernel_id: KernelId,
        layouts: &BTreeMap<ItemId, LayoutId>,
        symbol_for: impl Fn(&Program, ItemId) -> String,
        symbol_table: &mut BTreeMap<Box<str>, usize>,
    ) -> Result<Vec<JitDataSlot>, CodegenErrors> {
        let mut slots = Vec::with_capacity(layouts.len());
        let mut errors = Vec::new();
        for (&item, &layout) in layouts {
            match self.field_abi_shape(kernel_id, layout, "JIT imported data slot") {
                Ok(abi) => {
                    let mut cell = vec![0u8; abi.size as usize].into_boxed_slice();
                    let symbol: Box<str> = symbol_for(self.program, item).into_boxed_str();
                    symbol_table.insert(symbol.clone(), cell.as_mut_ptr() as usize);
                    slots.push(JitDataSlot { item, layout, cell });
                }
                Err(error) => errors.push(error),
            }
        }
        if errors.is_empty() {
            Ok(slots)
        } else {
            Err(CodegenErrors::new(errors))
        }
    }

    fn build_jit_call_signature(&self, kernel_id: KernelId) -> Result<CallSignature, CodegenError> {
        let kernel = &self.program.kernels()[kernel_id];
        let mut args = Vec::with_capacity(kernel.convention.parameters.len());
        for (index, parameter) in kernel.convention.parameters.iter().enumerate() {
            args.push(self.jit_abi_value_kind_for_pass(
                kernel_id,
                parameter.layout,
                parameter.pass_mode,
                &format!("JIT entry parameter {index}"),
            )?);
        }
        let result = self.jit_abi_value_kind_for_pass(
            kernel_id,
            kernel.convention.result.layout,
            kernel.convention.result.pass_mode,
            "JIT entry result",
        )?;
        Ok(CallSignature::new(args, result))
    }

    fn jit_abi_value_kind_for_pass(
        &self,
        kernel_id: KernelId,
        layout: LayoutId,
        pass: AbiPassMode,
        detail: &str,
    ) -> Result<AbiValueKind, CodegenError> {
        match pass {
            AbiPassMode::ByReference { .. } => Ok(AbiValueKind::Pointer),
            AbiPassMode::ByValue { .. } => {
                let abi = self.field_abi_shape(kernel_id, layout, detail)?;
                match abi.ty {
                    types::I8 => Ok(AbiValueKind::I8),
                    types::I64 => Ok(AbiValueKind::I64),
                    types::I128 => Ok(AbiValueKind::I128),
                    types::F64 => Ok(AbiValueKind::F64),
                    other => Err(CodegenError::UnsupportedLayout {
                        kernel: kernel_id,
                        layout,
                        detail: format!("{detail} lowers to unsupported JIT ABI type {other}")
                            .into(),
                    }),
                }
            }
        }
    }
}

