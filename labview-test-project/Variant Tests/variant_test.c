/*
 * C test DLL for probing undocumented LabVIEW Variant API functions.
 * Build with (from VS Developer Command Prompt):
 *   cl /LD variant_test.c /Fe:variant_test.dll
 *
 * No LabVIEW SDK headers or libs needed — all functions resolved at runtime.
 */

#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <windows.h>

#ifdef _WIN32
#define EXPORT __declspec(dllexport)
#else
#define EXPORT __attribute__((visibility("default")))
#endif

/* Function pointer types for LvVariant* functions.
 * Prototypes are guessed from function names — we probe to verify. */
typedef void*    (__cdecl *LvVariantGetDataPtr_t)(void *varhndl);
typedef void*    (__cdecl *LvVariantGetType_t)(void *varhndl);
typedef int32_t  (__cdecl *LvVariantIsEmpty_t)(void *varhndl);
typedef size_t   (__cdecl *LvVariantGetCompleteDataSize_t)(void *varhndl);
typedef void*    (__cdecl *LvVariantGetContents_t)(void *varhndl);
typedef void*    (__cdecl *GetTypeFromLvVariant_t)(void *varhndl);
typedef int32_t  (__cdecl *LvVariantCompare_t)(void *var1, void *var2);
typedef void*    (__cdecl *GetVariantPtrIfValid_t)(void *varhndl);

static FARPROC resolve_export(const char *name) {
    HMODULE hSelf = GetModuleHandle(NULL);
    FARPROC fn = GetProcAddress(hSelf, name);
    if (fn) return fn;

    HMODULE hLV = GetModuleHandle("labview");
    if (hLV) {
        fn = GetProcAddress(hLV, name);
        if (fn) return fn;
    }

    HMODULE hRT = GetModuleHandle("lvrt");
    if (hRT) {
        fn = GetProcAddress(hRT, name);
        if (fn) return fn;
    }

    return NULL;
}

static LvVariantGetDataPtr_t resolve_fn(void) {
    return (LvVariantGetDataPtr_t)resolve_export("LvVariantGetDataPtr");
}

static FILE* open_log(void) {
    const char *path = "C:\\Temp\\variant_c_test.log";
    return fopen(path, "a");
}

/*
 * test_variant_c_read_i32
 *
 * LabVIEW CLFN setup:
 *   variant: Variant, Adapt to Type, Handles by Value
 *   result:  I32, Pointer to Value
 *   return:  I32
 *
 * With "Handles by Value", LabVIEW passes the handle value directly.
 * A LabVIEW handle is T** (pointer to pointer to data), so the parameter
 * arrives as void** — NOT void*** (that would be "Pointers to Handles").
 *
 * We accept as void* to be maximally safe and cast as needed.
 */
EXPORT int32_t test_variant_c_read_i32(void* variant_raw, int32_t* result)
{
    FILE *f = open_log();
    if (!f) return -1;

    fprintf(f, "=== test_variant_c_read_i32 called ===\n");
    fprintf(f, "variant_raw (void*):      %p\n", variant_raw);

    if (!variant_raw) {
        fprintf(f, "ERROR: variant_raw is NULL\n");
        fclose(f);
        return 1;
    }

    /* Resolve LvVariantGetDataPtr at runtime */
    LvVariantGetDataPtr_t LvVariantGetDataPtr = resolve_fn();
    if (!LvVariantGetDataPtr) {
        fprintf(f, "ERROR: Could not resolve LvVariantGetDataPtr\n");
        fclose(f);
        return 5;
    }
    fprintf(f, "LvVariantGetDataPtr resolved at: %p\n", (void*)LvVariantGetDataPtr);

    int i;

    /*
     * Try 1: pass variant_raw directly (Handles by Value = the handle itself)
     * This is what our Rust code does with UHandle.
     */
    fprintf(f, "\n--- Try 1: LvVariantGetDataPtr(variant_raw) ---\n");
    void *ret1 = LvVariantGetDataPtr(variant_raw);
    fprintf(f, "  returned: %p\n", ret1);
    if (ret1 && (size_t)ret1 != 1 && (size_t)ret1 > 0x10000) {
        uint8_t *b = (uint8_t*)ret1;
        fprintf(f, "  bytes: ");
        for (i = 0; i < 32; i++) fprintf(f, "%02x ", b[i]);
        fprintf(f, "\n");
        fprintf(f, "  as i32: %d (0x%08x)\n", *(int32_t*)ret1, *(int32_t*)ret1);
    } else if ((size_t)ret1 == 1) {
        fprintf(f, "  => SENTINEL 1 (broken variant)\n");
    } else if (ret1 == NULL) {
        fprintf(f, "  => NULL (empty variant)\n");
    }

    /*
     * Try 2: dereference once first, then pass.
     * If variant_raw is actually void** (handle), then *variant_raw is void*.
     * h5labview does: LvVariantGetDataPtr(*hndl) where hndl is void***
     * but with "Handles by Value" we already have hndl=void**, so *hndl=void*.
     */
    fprintf(f, "\n--- Try 2: LvVariantGetDataPtr(*(void**)variant_raw) ---\n");
    void *deref1 = *(void**)variant_raw;
    fprintf(f, "  *(void**)variant_raw = %p\n", deref1);
    if (deref1 && (size_t)deref1 > 0x10000) {
        void *ret2 = LvVariantGetDataPtr(deref1);
        fprintf(f, "  returned: %p\n", ret2);
        if (ret2 && (size_t)ret2 != 1 && (size_t)ret2 > 0x10000) {
            uint8_t *b = (uint8_t*)ret2;
            fprintf(f, "  bytes: ");
            for (i = 0; i < 32; i++) fprintf(f, "%02x ", b[i]);
            fprintf(f, "\n");
            fprintf(f, "  as i32: %d (0x%08x)\n", *(int32_t*)ret2, *(int32_t*)ret2);
        } else if ((size_t)ret2 == 1) {
            fprintf(f, "  => SENTINEL 1 (broken variant)\n");
        } else if (ret2 == NULL) {
            fprintf(f, "  => NULL (empty variant)\n");
        }
    } else {
        fprintf(f, "  deref is NULL or too small, skipping\n");
    }

    /*
     * Use Try 2: dereference handle once, then pass to LvVariantGetDataPtr.
     * This matches h5labview: LvVariantGetDataPtr(*hndl)
     */
    void *data = NULL;
    if (deref1 && (size_t)deref1 > 0x10000) {
        void *ret2 = LvVariantGetDataPtr(deref1);
        if (ret2 && (size_t)ret2 != 1 && (size_t)ret2 > 0x10000) {
            data = ret2;
            fprintf(f, "\nUsing Try 2 (dereference + call) as data pointer.\n");
        }
    }

    if (data) {
        *result = *(int32_t*)data;
        fprintf(f, "Final result: %d (0x%08x)\n", *result, *result);
    } else {
        *result = -9999;
        fprintf(f, "No valid data pointer found!\n");
    }

    fprintf(f, "Done.\n\n");
    fflush(f);
    fclose(f);
    return 0;
}

/*
 * probe_variant_typedesc
 *
 * Extracts the type descriptor from any variant via GetTypeFromLvVariant
 * and dumps the bytes. Test with I32, DBL, Bool, String, Cluster, Array.
 *
 * LabVIEW CLFN setup:
 *   variant: Variant, Adapt to Type, Handles by Value
 *   return:  I32
 */
EXPORT int32_t probe_variant_typedesc(void* variant_raw)
{
    FILE *f = open_log();
    if (!f) return -1;

    fprintf(f, "========================================\n");
    fprintf(f, "=== probe_variant_typedesc called ===\n");
    fprintf(f, "variant_raw (handle):     %p\n", variant_raw);
    fflush(f);

    if (!variant_raw) {
        fprintf(f, "ERROR: variant_raw is NULL\n");
        fclose(f);
        return 1;
    }

    void *deref = *(void**)variant_raw;
    fprintf(f, "*variant_raw (inner):     %p\n", deref);
    fflush(f);

    if (!deref) {
        fprintf(f, "ERROR: *variant_raw is NULL\n");
        fclose(f);
        return 2;
    }

    int i;

    /* --- GetTypeFromLvVariant --- */
    GetTypeFromLvVariant_t fnGetType = (GetTypeFromLvVariant_t)resolve_export("GetTypeFromLvVariant");
    if (!fnGetType) {
        fprintf(f, "ERROR: GetTypeFromLvVariant not found\n");
        fclose(f);
        return 3;
    }

    __try {
        void *td = fnGetType(deref);
        fprintf(f, "GetTypeFromLvVariant(*handle): %p\n", td);
        fflush(f);

        if (!td || (size_t)td < 0x10000) {
            fprintf(f, "ERROR: returned invalid pointer\n");
            fclose(f);
            return 4;
        }

        uint8_t *b = (uint8_t*)td;

        /* First 2 bytes = size (LE u16), next 2 bytes = type code (LE u16) */
        uint16_t td_size = b[0] | (b[1] << 8);
        uint16_t td_code = b[2] | (b[3] << 8);
        fprintf(f, "Type descriptor size: %u bytes\n", td_size);
        fprintf(f, "Type code: 0x%04x (%u)\n", td_code, td_code);

        /* Dump exactly td_size bytes (capped at 256 for safety) */
        int dump_len = td_size;
        if (dump_len > 256) dump_len = 256;
        if (dump_len < 4) dump_len = 4;

        fprintf(f, "Raw bytes (%d): ", dump_len);
        for (i = 0; i < dump_len; i++) fprintf(f, "%02x ", b[i]);
        fprintf(f, "\n");

        /* Also dump as 16-bit words in hex (matching NI doc format) */
        fprintf(f, "As 16-bit LE words: ");
        for (i = 0; i + 1 < dump_len; i += 2) {
            uint16_t w = b[i] | (b[i+1] << 8);
            fprintf(f, "%04x ", w);
        }
        fprintf(f, "\n");

        /* Interpret the type code */
        fprintf(f, "Type interpretation: ");
        switch (td_code & 0xFF) {
            case 0x01: fprintf(f, "I8"); break;
            case 0x02: fprintf(f, "I16"); break;
            case 0x03: fprintf(f, "I32"); break;
            case 0x04: fprintf(f, "I64"); break;
            case 0x05: fprintf(f, "U8"); break;
            case 0x06: fprintf(f, "U16"); break;
            case 0x07: fprintf(f, "U32"); break;
            case 0x08: fprintf(f, "U64"); break;
            case 0x09: fprintf(f, "SGL"); break;
            case 0x0A: fprintf(f, "DBL"); break;
            case 0x0B: fprintf(f, "EXT"); break;
            case 0x0C: fprintf(f, "CSgl"); break;
            case 0x0D: fprintf(f, "CDbl"); break;
            case 0x0E: fprintf(f, "CExt"); break;
            case 0x15: fprintf(f, "Enum U8"); break;
            case 0x16: fprintf(f, "Enum U16"); break;
            case 0x17: fprintf(f, "Enum U32"); break;
            case 0x21: fprintf(f, "Boolean"); break;
            case 0x30: fprintf(f, "String"); break;
            case 0x32: fprintf(f, "Path"); break;
            case 0x40: fprintf(f, "Array"); break;
            case 0x50: fprintf(f, "Cluster"); break;
            case 0x53: fprintf(f, "Variant"); break;
            case 0x54: fprintf(f, "Waveform"); break;
            default:   fprintf(f, "Unknown (0x%02x)", td_code & 0xFF); break;
        }
        fprintf(f, "\n");

        /* For clusters, dump element count */
        if ((td_code & 0xFF) == 0x50 && td_size >= 6) {
            uint16_t n_elems = b[4] | (b[5] << 8);
            fprintf(f, "Cluster element count: %u\n", n_elems);
            /* Dump nested type descriptors */
            int offset = 6;
            int elem;
            for (elem = 0; elem < n_elems && offset + 4 <= dump_len; elem++) {
                uint16_t elem_size = b[offset] | (b[offset+1] << 8);
                uint16_t elem_code = b[offset+2] | (b[offset+3] << 8);
                fprintf(f, "  Element %d: size=%u, code=0x%04x", elem, elem_size, elem_code);
                switch (elem_code & 0xFF) {
                    case 0x03: fprintf(f, " (I32)"); break;
                    case 0x0A: fprintf(f, " (DBL)"); break;
                    case 0x21: fprintf(f, " (Boolean)"); break;
                    case 0x30: fprintf(f, " (String)"); break;
                    default: break;
                }
                fprintf(f, "\n");
                offset += elem_size;
            }
        }

        /* For arrays, dump dimension count and element type */
        if ((td_code & 0xFF) == 0x40 && td_size >= 6) {
            uint16_t n_dims = b[4] | (b[5] << 8);
            fprintf(f, "Array dimensions: %u\n", n_dims);
            /* Each dim is 4 bytes (i32), then element type descriptor follows */
            int elem_offset = 6 + n_dims * 4;
            if (elem_offset + 4 <= dump_len) {
                uint16_t elem_size = b[elem_offset] | (b[elem_offset+1] << 8);
                uint16_t elem_code = b[elem_offset+2] | (b[elem_offset+3] << 8);
                fprintf(f, "  Element type: size=%u, code=0x%04x\n", elem_size, elem_code);
            }
        }

        fflush(f);

    } __except(EXCEPTION_EXECUTE_HANDLER) {
        fprintf(f, "CRASHED! code=0x%08lx\n", GetExceptionCode());
        fflush(f);
        fclose(f);
        return 5;
    }

    /* Also get data pointer and dump a preview */
    fprintf(f, "\nData preview:\n"); fflush(f);
    LvVariantGetDataPtr_t fnDataPtr = resolve_fn();
    if (fnDataPtr && deref) {
        __try {
            void *dp = fnDataPtr(deref);
            fprintf(f, "  GetDataPtr(*handle): %p\n", dp);
            if (dp && (size_t)dp > 0x10000) {
                uint8_t *db = (uint8_t*)dp;
                fprintf(f, "  first 32 bytes: ");
                for (i = 0; i < 32; i++) fprintf(f, "%02x ", db[i]);
                fprintf(f, "\n");
            }
            fflush(f);
        } __except(EXCEPTION_EXECUTE_HANDLER) {
            fprintf(f, "  CRASHED! code=0x%08lx\n", GetExceptionCode()); fflush(f);
        }
    }

    fprintf(f, "\nDone.\n\n");
    fflush(f);
    fclose(f);
    return 0;
}

/*
 * probe_variant_api
 *
 * Probes undocumented LvVariant* functions to discover their behavior.
 * Each call is wrapped in SEH (__try/__except) so one crash doesn't
 * prevent the rest from being tested.
 *
 * Pass a Variant containing an I32 value so we can cross-reference results.
 *
 * LabVIEW CLFN setup:
 *   variant: Variant, Adapt to Type, Handles by Value
 *   return:  I32
 */
EXPORT int32_t probe_variant_api(void* variant_raw)
{
    FILE *f = open_log();
    if (!f) return -1;

    fprintf(f, "========================================\n");
    fprintf(f, "=== probe_variant_api called ===\n");
    fprintf(f, "variant_raw (handle):     %p\n", variant_raw);
    fflush(f);

    if (!variant_raw) {
        fprintf(f, "ERROR: variant_raw is NULL\n");
        fclose(f);
        return 1;
    }

    /* Dereference once to get inner pointer (for functions that need *hndl) */
    void *deref = *(void**)variant_raw;
    fprintf(f, "*variant_raw (inner):     %p\n", deref);
    fflush(f);

    int i;

    /* --- LvVariantIsEmpty --- */
    fprintf(f, "\n--- LvVariantIsEmpty ---\n"); fflush(f);
    LvVariantIsEmpty_t fnIsEmpty = (LvVariantIsEmpty_t)resolve_export("LvVariantIsEmpty");
    if (fnIsEmpty) {
        fprintf(f, "  resolved at: %p\n", (void*)fnIsEmpty); fflush(f);
        __try {
            int32_t r1 = fnIsEmpty(variant_raw);
            fprintf(f, "  IsEmpty(handle):  %d (0x%08x)\n", r1, r1); fflush(f);
        } __except(EXCEPTION_EXECUTE_HANDLER) {
            fprintf(f, "  CRASHED with handle! code=0x%08lx\n", GetExceptionCode()); fflush(f);
        }
        if (deref) {
            __try {
                int32_t r2 = fnIsEmpty(deref);
                fprintf(f, "  IsEmpty(*handle): %d (0x%08x)\n", r2, r2); fflush(f);
            } __except(EXCEPTION_EXECUTE_HANDLER) {
                fprintf(f, "  CRASHED with *handle! code=0x%08lx\n", GetExceptionCode()); fflush(f);
            }
        }
    } else {
        fprintf(f, "  NOT FOUND\n"); fflush(f);
    }

    /* --- LvVariantGetCompleteDataSize --- */
    fprintf(f, "\n--- LvVariantGetCompleteDataSize ---\n"); fflush(f);
    LvVariantGetCompleteDataSize_t fnSize = (LvVariantGetCompleteDataSize_t)resolve_export("LvVariantGetCompleteDataSize");
    if (fnSize) {
        fprintf(f, "  resolved at: %p\n", (void*)fnSize); fflush(f);
        __try {
            size_t s1 = fnSize(variant_raw);
            fprintf(f, "  size(handle):  %llu (0x%llx)\n", (unsigned long long)s1, (unsigned long long)s1); fflush(f);
        } __except(EXCEPTION_EXECUTE_HANDLER) {
            fprintf(f, "  CRASHED with handle! code=0x%08lx\n", GetExceptionCode()); fflush(f);
        }
        if (deref) {
            __try {
                size_t s2 = fnSize(deref);
                fprintf(f, "  size(*handle): %llu (0x%llx)\n", (unsigned long long)s2, (unsigned long long)s2); fflush(f);
            } __except(EXCEPTION_EXECUTE_HANDLER) {
                fprintf(f, "  CRASHED with *handle! code=0x%08lx\n", GetExceptionCode()); fflush(f);
            }
        }
    } else {
        fprintf(f, "  NOT FOUND\n"); fflush(f);
    }

    /* --- LvVariantGetType --- */
    fprintf(f, "\n--- LvVariantGetType ---\n"); fflush(f);
    LvVariantGetType_t fnGetType = (LvVariantGetType_t)resolve_export("LvVariantGetType");
    if (fnGetType) {
        fprintf(f, "  resolved at: %p\n", (void*)fnGetType); fflush(f);
        __try {
            void *t1 = fnGetType(variant_raw);
            fprintf(f, "  GetType(handle):  %p\n", t1); fflush(f);
            if (t1 && (size_t)t1 > 0x10000) {
                uint8_t *b = (uint8_t*)t1;
                fprintf(f, "  bytes: ");
                for (i = 0; i < 32; i++) fprintf(f, "%02x ", b[i]);
                fprintf(f, "\n");
                uint16_t td_size = (b[0] << 8) | b[1];
                uint16_t td_code = (b[2] << 8) | b[3];
                fprintf(f, "  as type desc: size=%u, code=0x%04x\n", td_size, td_code);
                fflush(f);
            }
        } __except(EXCEPTION_EXECUTE_HANDLER) {
            fprintf(f, "  CRASHED with handle! code=0x%08lx\n", GetExceptionCode()); fflush(f);
        }
        if (deref) {
            __try {
                void *t2 = fnGetType(deref);
                fprintf(f, "  GetType(*handle): %p\n", t2); fflush(f);
                if (t2 && (size_t)t2 > 0x10000) {
                    uint8_t *b = (uint8_t*)t2;
                    fprintf(f, "  bytes: ");
                    for (i = 0; i < 32; i++) fprintf(f, "%02x ", b[i]);
                    fprintf(f, "\n");
                    uint16_t td_size = (b[0] << 8) | b[1];
                    uint16_t td_code = (b[2] << 8) | b[3];
                    fprintf(f, "  as type desc: size=%u, code=0x%04x\n", td_size, td_code);
                    fflush(f);
                }
            } __except(EXCEPTION_EXECUTE_HANDLER) {
                fprintf(f, "  CRASHED with *handle! code=0x%08lx\n", GetExceptionCode()); fflush(f);
            }
        }
    } else {
        fprintf(f, "  NOT FOUND\n"); fflush(f);
    }

    /* --- GetTypeFromLvVariant --- */
    fprintf(f, "\n--- GetTypeFromLvVariant ---\n"); fflush(f);
    GetTypeFromLvVariant_t fnGetType2 = (GetTypeFromLvVariant_t)resolve_export("GetTypeFromLvVariant");
    if (fnGetType2) {
        fprintf(f, "  resolved at: %p\n", (void*)fnGetType2); fflush(f);
        __try {
            void *t1 = fnGetType2(variant_raw);
            fprintf(f, "  GetTypeFromLvVariant(handle):  %p\n", t1); fflush(f);
            if (t1 && (size_t)t1 > 0x10000) {
                uint8_t *b = (uint8_t*)t1;
                fprintf(f, "  bytes: ");
                for (i = 0; i < 32; i++) fprintf(f, "%02x ", b[i]);
                fprintf(f, "\n"); fflush(f);
            }
        } __except(EXCEPTION_EXECUTE_HANDLER) {
            fprintf(f, "  CRASHED with handle! code=0x%08lx\n", GetExceptionCode()); fflush(f);
        }
        if (deref) {
            __try {
                void *t2 = fnGetType2(deref);
                fprintf(f, "  GetTypeFromLvVariant(*handle): %p\n", t2); fflush(f);
                if (t2 && (size_t)t2 > 0x10000) {
                    uint8_t *b = (uint8_t*)t2;
                    fprintf(f, "  bytes: ");
                    for (i = 0; i < 32; i++) fprintf(f, "%02x ", b[i]);
                    fprintf(f, "\n"); fflush(f);
                }
            } __except(EXCEPTION_EXECUTE_HANDLER) {
                fprintf(f, "  CRASHED with *handle! code=0x%08lx\n", GetExceptionCode()); fflush(f);
            }
        }
    } else {
        fprintf(f, "  NOT FOUND\n"); fflush(f);
    }

    /* --- GetVariantPtrIfValid --- */
    fprintf(f, "\n--- GetVariantPtrIfValid ---\n"); fflush(f);
    GetVariantPtrIfValid_t fnValid = (GetVariantPtrIfValid_t)resolve_export("GetVariantPtrIfValid");
    if (fnValid) {
        fprintf(f, "  resolved at: %p\n", (void*)fnValid); fflush(f);
        __try {
            void *v1 = fnValid(variant_raw);
            fprintf(f, "  Valid(handle):  %p\n", v1); fflush(f);
            if (v1 && (size_t)v1 > 0x10000) {
                uint8_t *b = (uint8_t*)v1;
                fprintf(f, "  bytes: ");
                for (i = 0; i < 64; i++) fprintf(f, "%02x ", b[i]);
                fprintf(f, "\n"); fflush(f);
            }
        } __except(EXCEPTION_EXECUTE_HANDLER) {
            fprintf(f, "  CRASHED with handle! code=0x%08lx\n", GetExceptionCode()); fflush(f);
        }
        if (deref) {
            __try {
                void *v2 = fnValid(deref);
                fprintf(f, "  Valid(*handle): %p\n", v2); fflush(f);
                if (v2 && (size_t)v2 > 0x10000) {
                    uint8_t *b = (uint8_t*)v2;
                    fprintf(f, "  bytes: ");
                    for (i = 0; i < 64; i++) fprintf(f, "%02x ", b[i]);
                    fprintf(f, "\n"); fflush(f);
                }
            } __except(EXCEPTION_EXECUTE_HANDLER) {
                fprintf(f, "  CRASHED with *handle! code=0x%08lx\n", GetExceptionCode()); fflush(f);
            }
        }
    } else {
        fprintf(f, "  NOT FOUND\n"); fflush(f);
    }

    /* --- LvVariantGetContents --- */
    fprintf(f, "\n--- LvVariantGetContents ---\n"); fflush(f);
    LvVariantGetContents_t fnContents = (LvVariantGetContents_t)resolve_export("LvVariantGetContents");
    if (fnContents) {
        fprintf(f, "  resolved at: %p\n", (void*)fnContents); fflush(f);
        __try {
            void *c1 = fnContents(variant_raw);
            fprintf(f, "  Contents(handle):  %p\n", c1); fflush(f);
            if (c1 && (size_t)c1 > 0x10000) {
                uint8_t *b = (uint8_t*)c1;
                fprintf(f, "  bytes: ");
                for (i = 0; i < 32; i++) fprintf(f, "%02x ", b[i]);
                fprintf(f, "\n"); fflush(f);
            }
        } __except(EXCEPTION_EXECUTE_HANDLER) {
            fprintf(f, "  CRASHED with handle! code=0x%08lx\n", GetExceptionCode()); fflush(f);
        }
        if (deref) {
            __try {
                void *c2 = fnContents(deref);
                fprintf(f, "  Contents(*handle): %p\n", c2); fflush(f);
                if (c2 && (size_t)c2 > 0x10000) {
                    uint8_t *b = (uint8_t*)c2;
                    fprintf(f, "  bytes: ");
                    for (i = 0; i < 32; i++) fprintf(f, "%02x ", b[i]);
                    fprintf(f, "\n"); fflush(f);
                }
            } __except(EXCEPTION_EXECUTE_HANDLER) {
                fprintf(f, "  CRASHED with *handle! code=0x%08lx\n", GetExceptionCode()); fflush(f);
            }
        }
    } else {
        fprintf(f, "  NOT FOUND\n"); fflush(f);
    }

    /* --- LvVariantGetDataPtr (reference, known working) --- */
    fprintf(f, "\n--- LvVariantGetDataPtr (reference) ---\n"); fflush(f);
    LvVariantGetDataPtr_t fnDataPtr = resolve_fn();
    if (fnDataPtr && deref) {
        __try {
            void *dp = fnDataPtr(deref);
            fprintf(f, "  GetDataPtr(*handle): %p\n", dp); fflush(f);
            if (dp && (size_t)dp > 0x10000) {
                fprintf(f, "  *(int32_t*)dp = %d\n", *(int32_t*)dp); fflush(f);
            }
        } __except(EXCEPTION_EXECUTE_HANDLER) {
            fprintf(f, "  CRASHED! code=0x%08lx\n", GetExceptionCode()); fflush(f);
        }
    }

    fprintf(f, "\nDone.\n\n");
    fflush(f);
    fclose(f);
    return 0;
}
