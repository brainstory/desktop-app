#include <include/whisper.h>
// Brainstory: ggml.h comes from llama-cpp-sys-2's ggml (see build.rs)
#include <ggml.h>

#ifdef GGML_USE_VULKAN
#include "ggml-vulkan.h"
#endif
