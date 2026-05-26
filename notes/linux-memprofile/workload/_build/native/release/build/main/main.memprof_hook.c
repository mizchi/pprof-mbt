// Statically linked into a `moon-pprof memprofile-native`-instrumented
// MoonBit native binary. The driver patches the generated `main.c` so
// that `moonbit_malloc_inlined(size)` calls `__moon_pprof_alloc_hook(size)`
// before the real allocator runs. We capture a backtrace, resolve each
// frame via `dladdr` (in-process — symbols are still loaded), and
// append a binary record to `$MOON_PPROF_RAW_OUTPUT`. The Rust side
// aggregates and emits the gzip'd pprof.
//
// Record format (binary stream, little-endian, host byte order):
//   for each sampled alloc:
//     u64  size          // bytes requested, pre-scaling
//     u8   nframes       // number of frames that follow, ≤ 64
//     for each frame:
//       u16 name_len     // bytes in symbol name
//       u8  name[name_len]  // dladdr symbol name (or "0x<addr>" fallback)
//
// Scaling for 1/N sampling is applied on the Rust side based on
// $MOON_PPROF_SAMPLE_RATE.

#define _GNU_SOURCE
#include <dlfcn.h>
#include <execinfo.h>
#include <pthread.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static __thread int mpprof_in_hook = 0;

static FILE *mpprof_out = NULL;
static uint64_t mpprof_counter = 0;
static uint64_t mpprof_rate = 1;
static pthread_mutex_t mpprof_mu = PTHREAD_MUTEX_INITIALIZER;

__attribute__((constructor))
static void mpprof_init(void) {
  const char *path = getenv("MOON_PPROF_RAW_OUTPUT");
  if (path && *path) {
    mpprof_out = fopen(path, "wb");
  }
  const char *rate = getenv("MOON_PPROF_SAMPLE_RATE");
  if (rate && *rate) {
    char *end = NULL;
    unsigned long long v = strtoull(rate, &end, 10);
    if (v >= 1) {
      mpprof_rate = (uint64_t)v;
    }
  }
}

__attribute__((destructor))
static void mpprof_fini(void) {
  // Flush + close. The driver opens the file post-exit; even a
  // partial write is parseable since records are self-delimiting.
  if (mpprof_out) {
    fflush(mpprof_out);
    fclose(mpprof_out);
    mpprof_out = NULL;
  }
}

// Public symbol called by the patched moonbit_malloc_inlined. Marked
// weak so binaries built *without* the relink path (e.g. plain
// `moon build`) still link cleanly even if the hook .o was forgotten.
__attribute__((weak)) void __moon_pprof_alloc_hook(size_t size);
void __moon_pprof_alloc_hook(size_t size) {
  if (mpprof_in_hook) {
    return;
  }
  if (mpprof_out == NULL) {
    return;
  }
  mpprof_in_hook = 1;

  pthread_mutex_lock(&mpprof_mu);
  uint64_t n = ++mpprof_counter;
  int do_sample = (mpprof_rate <= 1) || (((n - 1) % mpprof_rate) == 0);
  pthread_mutex_unlock(&mpprof_mu);
  if (!do_sample) {
    mpprof_in_hook = 0;
    return;
  }

  void *bt[64];
  int nframes = backtrace(bt, 64);
  if (nframes <= 0) {
    mpprof_in_hook = 0;
    return;
  }
  if (nframes > 255) nframes = 255;

  // Build one record in a stack buffer so we can write it atomically
  // under the mutex (otherwise concurrent threads would interleave).
  // 8192 covers 64 frames × ~120 bytes/name; we cap per-name length
  // at 250 to stay within budget.
  unsigned char buf[8192];
  size_t pos = 0;

  uint64_t bytes64 = (uint64_t)size;
  memcpy(buf + pos, &bytes64, sizeof(bytes64));
  pos += sizeof(bytes64);

  // Reserve the nframes byte; we'll backfill once we know how many
  // frames actually fit.
  size_t nframes_pos = pos++;

  int kept = 0;
  for (int i = 0; i < nframes; i++) {
    if (pos + 2 + 250 > sizeof(buf)) break;
    Dl_info di;
    memset(&di, 0, sizeof(di));
    const char *name = NULL;
    char fallback[32];
    if (dladdr(bt[i], &di) && di.dli_sname) {
      name = di.dli_sname;
    } else {
      snprintf(fallback, sizeof(fallback), "0x%lx",
               (unsigned long)(uintptr_t)bt[i]);
      name = fallback;
    }
    size_t name_len = strnlen(name, 250);
    uint16_t name_len16 = (uint16_t)name_len;
    memcpy(buf + pos, &name_len16, sizeof(name_len16));
    pos += sizeof(name_len16);
    memcpy(buf + pos, name, name_len);
    pos += name_len;
    kept++;
  }
  buf[nframes_pos] = (unsigned char)kept;

  pthread_mutex_lock(&mpprof_mu);
  if (mpprof_out) {
    fwrite(buf, 1, pos, mpprof_out);
  }
  pthread_mutex_unlock(&mpprof_mu);

  mpprof_in_hook = 0;
}
