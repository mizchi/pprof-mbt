#ifdef __cplusplus
extern "C" {
#endif

#include "moonbit.h"

#ifdef _MSC_VER
#define _Noreturn __declspec(noreturn)
#endif

#if defined(__clang__)
#pragma clang diagnostic ignored "-Wshift-op-parentheses"
#pragma clang diagnostic ignored "-Wtautological-compare"
#endif

MOONBIT_EXPORT _Noreturn void moonbit_panic(void);
MOONBIT_EXPORT void *moonbit_malloc_array(enum moonbit_block_kind kind,
                                          int elem_size_shift, int32_t len);
int memcmp(const void *s1, const void *s2, size_t n);
MOONBIT_EXPORT int moonbit_val_array_equal(const void *lhs, const void *rhs);
MOONBIT_EXPORT moonbit_string_t moonbit_add_string(moonbit_string_t s1,
                                                   moonbit_string_t s2);
MOONBIT_EXPORT void moonbit_unsafe_bytes_blit(moonbit_bytes_t dst,
                                              int32_t dst_start,
                                              moonbit_bytes_t src,
                                              int32_t src_offset, int32_t len);
MOONBIT_EXPORT moonbit_string_t moonbit_unsafe_bytes_sub_string(
    moonbit_bytes_t bytes, int32_t start, int32_t len);
MOONBIT_EXPORT int32_t moonbit_unsafe_val_array_blit(void *dst,
                                                     int32_t dst_offset,
                                                     void *src,
                                                     int32_t src_offset,
                                                     int32_t len,
                                                     int32_t elem_size);
MOONBIT_EXPORT int32_t moonbit_unsafe_ref_array_blit(void *dst,
                                                     int32_t dst_offset,
                                                     void *src,
                                                     int32_t src_offset,
                                                     int32_t len);
MOONBIT_EXPORT void moonbit_println(moonbit_string_t str);
MOONBIT_EXPORT moonbit_bytes_t *moonbit_get_cli_args(void);
MOONBIT_EXPORT void moonbit_runtime_init(int argc, char **argv);
MOONBIT_EXPORT void moonbit_drop_object(void *);
MOONBIT_EXPORT int32_t moonbit_utf16_len_from_utf8(moonbit_bytes_t src,
                                                   int32_t src_offset,
                                                   int32_t src_length);
MOONBIT_EXPORT int32_t moonbit_utf8_decode_into_utf16(
    moonbit_bytes_t src, int32_t src_offset, int32_t src_length,
    moonbit_string_t dst, int32_t dst_offset);
MOONBIT_EXPORT int32_t moonbit_utf8_decode_lossy_into_utf16(
    moonbit_bytes_t src, int32_t src_offset, int32_t src_length,
    moonbit_string_t dst, int32_t dst_offset);
MOONBIT_EXPORT int32_t moonbit_utf8_len_from_utf16(moonbit_string_t src,
                                                   int32_t src_offset,
                                                   int32_t src_length);
MOONBIT_EXPORT int32_t moonbit_utf8_encode_from_utf16(
    moonbit_string_t src, int32_t src_offset, int32_t src_length,
    moonbit_bytes_t dst, int32_t dst_offset);

#define Moonbit_make_regular_object_header(ptr_field_offset, ptr_field_count,  \
                                           tag)                                \
  (((uint32_t)moonbit_BLOCK_KIND_REGULAR << 30) |                              \
   (((uint32_t)(ptr_field_offset) & (((uint32_t)1 << 11) - 1)) << 19) |        \
   (((uint32_t)(ptr_field_count) & (((uint32_t)1 << 11) - 1)) << 8) |          \
   ((tag) & 0xFF))

// header manipulation macros
#define Moonbit_object_ptr_field_offset(obj)                                   \
  ((Moonbit_object_header(obj)->meta >> 19) & (((uint32_t)1 << 11) - 1))

#define Moonbit_object_ptr_field_count(obj)                                    \
  ((Moonbit_object_header(obj)->meta >> 8) & (((uint32_t)1 << 11) - 1))

#if !defined(_WIN64) && !defined(_WIN32)
void *malloc(size_t size);
void free(void *ptr);
#define libc_malloc malloc
#define libc_free free
#endif

// several important runtime functions are inlined
static void *moonbit_malloc_inlined(size_t size) {
  extern void __moon_pprof_alloc_hook(size_t);
  __moon_pprof_alloc_hook(size);
  struct moonbit_object *ptr = (struct moonbit_object *)libc_malloc(
        sizeof(struct moonbit_object) + size);
  ptr->rc = 1;
  return ptr + 1;
}

#define moonbit_malloc(obj) moonbit_malloc_inlined(obj)
#define moonbit_free(obj) libc_free(Moonbit_object_header(obj))

static void moonbit_incref_inlined(void *ptr) {
  struct moonbit_object *header = Moonbit_object_header(ptr);
  int32_t const count = header->rc;
  if (count > 0) {
    header->rc = count + 1;
  }
}

#define moonbit_incref moonbit_incref_inlined

static void moonbit_decref_inlined(void *ptr) {
  struct moonbit_object *header = Moonbit_object_header(ptr);
  int32_t const count = header->rc;
  if (count > 1) {
    header->rc = count - 1;
  } else if (count == 1) {
    moonbit_drop_object(ptr);
  }
}

#define moonbit_decref moonbit_decref_inlined

#define moonbit_unsafe_make_string moonbit_make_string

// detect whether compiler builtins exist for advanced bitwise operations
#ifdef __has_builtin

#if __has_builtin(__builtin_clz)
#define HAS_BUILTIN_CLZ
#endif

#if __has_builtin(__builtin_ctz)
#define HAS_BUILTIN_CTZ
#endif

#if __has_builtin(__builtin_popcount)
#define HAS_BUILTIN_POPCNT
#endif

#if __has_builtin(__builtin_sqrt)
#define HAS_BUILTIN_SQRT
#endif

#if __has_builtin(__builtin_sqrtf)
#define HAS_BUILTIN_SQRTF
#endif

#if __has_builtin(__builtin_fabs)
#define HAS_BUILTIN_FABS
#endif

#if __has_builtin(__builtin_fabsf)
#define HAS_BUILTIN_FABSF
#endif

#endif

// if there is no builtin operators, use software implementation
#ifdef HAS_BUILTIN_CLZ
static inline int32_t moonbit_clz32(int32_t x) {
  return x == 0 ? 32 : __builtin_clz(x);
}

static inline int32_t moonbit_clz64(int64_t x) {
  return x == 0 ? 64 : __builtin_clzll(x);
}

#undef HAS_BUILTIN_CLZ
#else
// table for [clz] value of 4bit integer.
static const uint8_t moonbit_clz4[] = {4, 3, 2, 2, 1, 1, 1, 1,
                                       0, 0, 0, 0, 0, 0, 0, 0};

int32_t moonbit_clz32(uint32_t x) {
  /* The ideas is to:

     1. narrow down the 4bit block where the most signficant "1" bit lies,
        using binary search
     2. find the number of leading zeros in that 4bit block via table lookup

     Different time/space tradeoff can be made here by enlarging the table
     and do less binary search.
     One benefit of the 4bit lookup table is that it can fit into a single cache
     line.
  */
  int32_t result = 0;
  if (x > 0xffff) {
    x >>= 16;
  } else {
    result += 16;
  }
  if (x > 0xff) {
    x >>= 8;
  } else {
    result += 8;
  }
  if (x > 0xf) {
    x >>= 4;
  } else {
    result += 4;
  }
  return result + moonbit_clz4[x];
}

int32_t moonbit_clz64(uint64_t x) {
  int32_t result = 0;
  if (x > 0xffffffff) {
    x >>= 32;
  } else {
    result += 32;
  }
  return result + moonbit_clz32((uint32_t)x);
}
#endif

#ifdef HAS_BUILTIN_CTZ
static inline int32_t moonbit_ctz32(int32_t x) {
  return x == 0 ? 32 : __builtin_ctz(x);
}

static inline int32_t moonbit_ctz64(int64_t x) {
  return x == 0 ? 64 : __builtin_ctzll(x);
}

#undef HAS_BUILTIN_CTZ
#else
int32_t moonbit_ctz32(int32_t x) {
  /* The algorithm comes from:

       Leiserson, Charles E. et al. “Using de Bruijn Sequences to Index a 1 in a
     Computer Word.” (1998).

     The ideas is:

     1. leave only the least significant "1" bit in the input,
        set all other bits to "0". This is achieved via [x & -x]
     2. now we have [x * n == n << ctz(x)], if [n] is a de bruijn sequence
        (every 5bit pattern occurn exactly once when you cycle through the bit
     string), we can find [ctz(x)] from the most significant 5 bits of [x * n]
 */
  static const uint32_t de_bruijn_32 = 0x077CB531;
  static const uint8_t index32[] = {0,  1,  28, 2,  29, 14, 24, 3,  30, 22, 20,
                                    15, 25, 17, 4,  8,  31, 27, 13, 23, 21, 19,
                                    16, 7,  26, 12, 18, 6,  11, 5,  10, 9};
  return (x == 0) * 32 + index32[(de_bruijn_32 * (x & -x)) >> 27];
}

int32_t moonbit_ctz64(int64_t x) {
  static const uint64_t de_bruijn_64 = 0x0218A392CD3D5DBF;
  static const uint8_t index64[] = {
      0,  1,  2,  7,  3,  13, 8,  19, 4,  25, 14, 28, 9,  34, 20, 40,
      5,  17, 26, 38, 15, 46, 29, 48, 10, 31, 35, 54, 21, 50, 41, 57,
      63, 6,  12, 18, 24, 27, 33, 39, 16, 37, 45, 47, 30, 53, 49, 56,
      62, 11, 23, 32, 36, 44, 52, 55, 61, 22, 43, 51, 60, 42, 59, 58};
  return (x == 0) * 64 + index64[(de_bruijn_64 * (x & -x)) >> 58];
}
#endif

#ifdef HAS_BUILTIN_POPCNT

#define moonbit_popcnt32 __builtin_popcount
#define moonbit_popcnt64 __builtin_popcountll
#undef HAS_BUILTIN_POPCNT

#else
int32_t moonbit_popcnt32(uint32_t x) {
  /* The classic SIMD Within A Register algorithm.
     ref: [https://nimrod.blog/posts/algorithms-behind-popcount/]
 */
  x = x - ((x >> 1) & 0x55555555);
  x = (x & 0x33333333) + ((x >> 2) & 0x33333333);
  x = (x + (x >> 4)) & 0x0F0F0F0F;
  return (x * 0x01010101) >> 24;
}

int32_t moonbit_popcnt64(uint64_t x) {
  x = x - ((x >> 1) & 0x5555555555555555);
  x = (x & 0x3333333333333333) + ((x >> 2) & 0x3333333333333333);
  x = (x + (x >> 4)) & 0x0F0F0F0F0F0F0F0F;
  return (x * 0x0101010101010101) >> 56;
}
#endif

/* The following sqrt implementation comes from
   [musl](https://git.musl-libc.org/cgit/musl),
   with some helpers inlined to make it zero dependency.
 */
#ifdef MOONBIT_NATIVE_NO_SYS_HEADER
const uint16_t __rsqrt_tab[128] = {
    0xb451, 0xb2f0, 0xb196, 0xb044, 0xaef9, 0xadb6, 0xac79, 0xab43, 0xaa14,
    0xa8eb, 0xa7c8, 0xa6aa, 0xa592, 0xa480, 0xa373, 0xa26b, 0xa168, 0xa06a,
    0x9f70, 0x9e7b, 0x9d8a, 0x9c9d, 0x9bb5, 0x9ad1, 0x99f0, 0x9913, 0x983a,
    0x9765, 0x9693, 0x95c4, 0x94f8, 0x9430, 0x936b, 0x92a9, 0x91ea, 0x912e,
    0x9075, 0x8fbe, 0x8f0a, 0x8e59, 0x8daa, 0x8cfe, 0x8c54, 0x8bac, 0x8b07,
    0x8a64, 0x89c4, 0x8925, 0x8889, 0x87ee, 0x8756, 0x86c0, 0x862b, 0x8599,
    0x8508, 0x8479, 0x83ec, 0x8361, 0x82d8, 0x8250, 0x81c9, 0x8145, 0x80c2,
    0x8040, 0xff02, 0xfd0e, 0xfb25, 0xf947, 0xf773, 0xf5aa, 0xf3ea, 0xf234,
    0xf087, 0xeee3, 0xed47, 0xebb3, 0xea27, 0xe8a3, 0xe727, 0xe5b2, 0xe443,
    0xe2dc, 0xe17a, 0xe020, 0xdecb, 0xdd7d, 0xdc34, 0xdaf1, 0xd9b3, 0xd87b,
    0xd748, 0xd61a, 0xd4f1, 0xd3cd, 0xd2ad, 0xd192, 0xd07b, 0xcf69, 0xce5b,
    0xcd51, 0xcc4a, 0xcb48, 0xca4a, 0xc94f, 0xc858, 0xc764, 0xc674, 0xc587,
    0xc49d, 0xc3b7, 0xc2d4, 0xc1f4, 0xc116, 0xc03c, 0xbf65, 0xbe90, 0xbdbe,
    0xbcef, 0xbc23, 0xbb59, 0xba91, 0xb9cc, 0xb90a, 0xb84a, 0xb78c, 0xb6d0,
    0xb617, 0xb560,
};

/* returns a*b*2^-32 - e, with error 0 <= e < 1.  */
static inline uint32_t mul32(uint32_t a, uint32_t b) {
  return (uint64_t)a * b >> 32;
}
#endif

#ifdef MOONBIT_NATIVE_NO_SYS_HEADER
float sqrtf(float x) {
  uint32_t ix, m, m1, m0, even, ey;

  ix = *(uint32_t *)&x;
  if (ix - 0x00800000 >= 0x7f800000 - 0x00800000) {
    /* x < 0x1p-126 or inf or nan.  */
    if (ix * 2 == 0)
      return x;
    if (ix == 0x7f800000)
      return x;
    if (ix > 0x7f800000)
      return (x - x) / (x - x);
    /* x is subnormal, normalize it.  */
    x *= 0x1p23f;
    ix = *(uint32_t *)&x;
    ix -= 23 << 23;
  }

  /* x = 4^e m; with int e and m in [1, 4).  */
  even = ix & 0x00800000;
  m1 = (ix << 8) | 0x80000000;
  m0 = (ix << 7) & 0x7fffffff;
  m = even ? m0 : m1;

  /* 2^e is the exponent part of the return value.  */
  ey = ix >> 1;
  ey += 0x3f800000 >> 1;
  ey &= 0x7f800000;

  /* compute r ~ 1/sqrt(m), s ~ sqrt(m) with 2 goldschmidt iterations.  */
  static const uint32_t three = 0xc0000000;
  uint32_t r, s, d, u, i;
  i = (ix >> 17) % 128;
  r = (uint32_t)__rsqrt_tab[i] << 16;
  /* |r*sqrt(m) - 1| < 0x1p-8 */
  s = mul32(m, r);
  /* |s/sqrt(m) - 1| < 0x1p-8 */
  d = mul32(s, r);
  u = three - d;
  r = mul32(r, u) << 1;
  /* |r*sqrt(m) - 1| < 0x1.7bp-16 */
  s = mul32(s, u) << 1;
  /* |s/sqrt(m) - 1| < 0x1.7bp-16 */
  d = mul32(s, r);
  u = three - d;
  s = mul32(s, u);
  /* -0x1.03p-28 < s/sqrt(m) - 1 < 0x1.fp-31 */
  s = (s - 1) >> 6;
  /* s < sqrt(m) < s + 0x1.08p-23 */

  /* compute nearest rounded result.  */
  uint32_t d0, d1, d2;
  float y, t;
  d0 = (m << 16) - s * s;
  d1 = s - d0;
  d2 = d1 + s + 1;
  s += d1 >> 31;
  s &= 0x007fffff;
  s |= ey;
  y = *(float *)&s;
  /* handle rounding and inexact exception. */
  uint32_t tiny = d2 == 0 ? 0 : 0x01000000;
  tiny |= (d1 ^ d2) & 0x80000000;
  t = *(float *)&tiny;
  y = y + t;
  return y;
}
#endif

#ifdef MOONBIT_NATIVE_NO_SYS_HEADER
/* returns a*b*2^-64 - e, with error 0 <= e < 3.  */
static inline uint64_t mul64(uint64_t a, uint64_t b) {
  uint64_t ahi = a >> 32;
  uint64_t alo = a & 0xffffffff;
  uint64_t bhi = b >> 32;
  uint64_t blo = b & 0xffffffff;
  return ahi * bhi + (ahi * blo >> 32) + (alo * bhi >> 32);
}

double sqrt(double x) {
  uint64_t ix, top, m;

  /* special case handling.  */
  ix = *(uint64_t *)&x;
  top = ix >> 52;
  if (top - 0x001 >= 0x7ff - 0x001) {
    /* x < 0x1p-1022 or inf or nan.  */
    if (ix * 2 == 0)
      return x;
    if (ix == 0x7ff0000000000000)
      return x;
    if (ix > 0x7ff0000000000000)
      return (x - x) / (x - x);
    /* x is subnormal, normalize it.  */
    x *= 0x1p52;
    ix = *(uint64_t *)&x;
    top = ix >> 52;
    top -= 52;
  }

  /* argument reduction:
     x = 4^e m; with integer e, and m in [1, 4)
     m: fixed point representation [2.62]
     2^e is the exponent part of the result.  */
  int even = top & 1;
  m = (ix << 11) | 0x8000000000000000;
  if (even)
    m >>= 1;
  top = (top + 0x3ff) >> 1;

  /* approximate r ~ 1/sqrt(m) and s ~ sqrt(m) when m in [1,4)

     initial estimate:
     7bit table lookup (1bit exponent and 6bit significand).

     iterative approximation:
     using 2 goldschmidt iterations with 32bit int arithmetics
     and a final iteration with 64bit int arithmetics.

     details:

     the relative error (e = r0 sqrt(m)-1) of a linear estimate
     (r0 = a m + b) is |e| < 0.085955 ~ 0x1.6p-4 at best,
     a table lookup is faster and needs one less iteration
     6 bit lookup table (128b) gives |e| < 0x1.f9p-8
     7 bit lookup table (256b) gives |e| < 0x1.fdp-9
     for single and double prec 6bit is enough but for quad
     prec 7bit is needed (or modified iterations). to avoid
     one more iteration >=13bit table would be needed (16k).

     a newton-raphson iteration for r is
       w = r*r
       u = 3 - m*w
       r = r*u/2
     can use a goldschmidt iteration for s at the end or
       s = m*r

     first goldschmidt iteration is
       s = m*r
       u = 3 - s*r
       r = r*u/2
       s = s*u/2
     next goldschmidt iteration is
       u = 3 - s*r
       r = r*u/2
       s = s*u/2
     and at the end r is not computed only s.

     they use the same amount of operations and converge at the
     same quadratic rate, i.e. if
       r1 sqrt(m) - 1 = e, then
       r2 sqrt(m) - 1 = -3/2 e^2 - 1/2 e^3
     the advantage of goldschmidt is that the mul for s and r
     are independent (computed in parallel), however it is not
     "self synchronizing": it only uses the input m in the
     first iteration so rounding errors accumulate. at the end
     or when switching to larger precision arithmetics rounding
     errors dominate so the first iteration should be used.

     the fixed point representations are
       m: 2.30 r: 0.32, s: 2.30, d: 2.30, u: 2.30, three: 2.30
     and after switching to 64 bit
       m: 2.62 r: 0.64, s: 2.62, d: 2.62, u: 2.62, three: 2.62  */

  static const uint64_t three = 0xc0000000;
  uint64_t r, s, d, u, i;

  i = (ix >> 46) % 128;
  r = (uint32_t)__rsqrt_tab[i] << 16;
  /* |r sqrt(m) - 1| < 0x1.fdp-9 */
  s = mul32(m >> 32, r);
  /* |s/sqrt(m) - 1| < 0x1.fdp-9 */
  d = mul32(s, r);
  u = three - d;
  r = mul32(r, u) << 1;
  /* |r sqrt(m) - 1| < 0x1.7bp-16 */
  s = mul32(s, u) << 1;
  /* |s/sqrt(m) - 1| < 0x1.7bp-16 */
  d = mul32(s, r);
  u = three - d;
  r = mul32(r, u) << 1;
  /* |r sqrt(m) - 1| < 0x1.3704p-29 (measured worst-case) */
  r = r << 32;
  s = mul64(m, r);
  d = mul64(s, r);
  u = (three << 32) - d;
  s = mul64(s, u); /* repr: 3.61 */
  /* -0x1p-57 < s - sqrt(m) < 0x1.8001p-61 */
  s = (s - 2) >> 9; /* repr: 12.52 */
  /* -0x1.09p-52 < s - sqrt(m) < -0x1.fffcp-63 */

  /* s < sqrt(m) < s + 0x1.09p-52,
     compute nearest rounded result:
     the nearest result to 52 bits is either s or s+0x1p-52,
     we can decide by comparing (2^52 s + 0.5)^2 to 2^104 m.  */
  uint64_t d0, d1, d2;
  double y, t;
  d0 = (m << 42) - s * s;
  d1 = s - d0;
  d2 = d1 + s + 1;
  s += d1 >> 63;
  s &= 0x000fffffffffffff;
  s |= top << 52;
  y = *(double *)&s;
  return y;
}
#endif

#ifdef MOONBIT_NATIVE_NO_SYS_HEADER
double fabs(double x) {
  union {
    double f;
    uint64_t i;
  } u = {x};
  u.i &= 0x7fffffffffffffffULL;
  return u.f;
}
#endif

#ifdef MOONBIT_NATIVE_NO_SYS_HEADER
float fabsf(float x) {
  union {
    float f;
    uint32_t i;
  } u = {x};
  u.i &= 0x7fffffff;
  return u.f;
}
#endif

#ifdef _MSC_VER
/* MSVC treats syntactic division by zero as fatal error,
   even for float point numbers,
   so we have to use a constant variable to work around this */
static const int MOONBIT_ZERO = 0;
#else
#define MOONBIT_ZERO 0
#endif

#ifdef __cplusplus
}
#endif
struct _M0TPB13StringBuilder;

struct _M0TPB5ArrayGiE;

struct _M0TPB5ArrayGRPB5ArrayGiEE;

struct _M0TPB13StringBuilder {
  int32_t $1;
  uint16_t* $0;
  
};

struct _M0TPB5ArrayGiE {
  int32_t $1;
  int32_t* $0;
  
};

struct _M0TPB5ArrayGRPB5ArrayGiEE {
  int32_t $1;
  struct _M0TPB5ArrayGiE** $0;
  
};

int32_t _M0FP36mizchi26memprofile_2dlinux_2dcheck4main13alloc__arrays(
  int32_t
);

int32_t _M0FP36mizchi26memprofile_2dlinux_2dcheck4main14alloc__strings(
  int32_t
);

int32_t _M0FPB7printlnGsE(moonbit_string_t);

moonbit_string_t _M0IPC13int3IntPB4Show10to__string(int32_t);

int32_t _M0MPC15array5Array4pushGiE(struct _M0TPB5ArrayGiE*, int32_t);

int32_t _M0MPC15array5Array4pushGRPB5ArrayGiEE(
  struct _M0TPB5ArrayGRPB5ArrayGiEE*,
  struct _M0TPB5ArrayGiE*
);

int32_t _M0MPC15array5Array7reallocGiE(struct _M0TPB5ArrayGiE*);

int32_t _M0MPC15array5Array7reallocGRPB5ArrayGiEE(
  struct _M0TPB5ArrayGRPB5ArrayGiEE*
);

int32_t _M0MPC15array5Array14resize__bufferGiE(
  struct _M0TPB5ArrayGiE*,
  int32_t
);

int32_t _M0MPC15array5Array14resize__bufferGRPB5ArrayGiEE(
  struct _M0TPB5ArrayGRPB5ArrayGiEE*,
  int32_t
);

moonbit_string_t _M0MPC13int3Int18to__string_2einner(int32_t, int32_t);

int32_t _M0FPB14radix__count32(uint32_t, int32_t);

int32_t _M0FPB12hex__count32(uint32_t);

int32_t _M0FPB12dec__count32(uint32_t);

int32_t _M0FPB20int__to__string__dec(uint16_t*, uint32_t, int32_t, int32_t);

int32_t _M0FPB24int__to__string__generic(
  uint16_t*,
  uint32_t,
  int32_t,
  int32_t,
  int32_t
);

int32_t _M0FPB20int__to__string__hex(uint16_t*, uint32_t, int32_t, int32_t);

int32_t _M0IPB13StringBuilderPB6Logger13write__string(
  struct _M0TPB13StringBuilder*,
  moonbit_string_t
);

int32_t _M0MPC15array10FixedArray26unsafe__blit__from__string(
  uint16_t*,
  int32_t,
  moonbit_string_t,
  int32_t,
  int32_t
);

int32_t _M0MPB13StringBuilder19grow__if__necessary(
  struct _M0TPB13StringBuilder*,
  int32_t
);

moonbit_string_t _M0MPB13StringBuilder10to__string(
  struct _M0TPB13StringBuilder*
);

struct _M0TPB13StringBuilder* _M0MPB13StringBuilder21StringBuilder_2einner(
  int32_t
);

int32_t _M0MPB18UninitializedArray12unsafe__blitGiE(
  int32_t*,
  int32_t,
  int32_t*,
  int32_t,
  int32_t
);

int32_t _M0MPB18UninitializedArray12unsafe__blitGRPB5ArrayGiEE(
  struct _M0TPB5ArrayGiE**,
  int32_t,
  struct _M0TPB5ArrayGiE**,
  int32_t,
  int32_t
);

int32_t _M0MPC15array10FixedArray12unsafe__blitGkE(
  uint16_t*,
  int32_t,
  uint16_t*,
  int32_t,
  int32_t
);

int32_t _M0MPC15array10FixedArray12unsafe__blitGRPB17UnsafeMaybeUninitGiEE(
  int32_t*,
  int32_t,
  int32_t*,
  int32_t,
  int32_t
);

int32_t _M0MPC15array10FixedArray12unsafe__blitGRPB17UnsafeMaybeUninitGRPB5ArrayGiEEE(
  struct _M0TPB5ArrayGiE**,
  int32_t,
  struct _M0TPB5ArrayGiE**,
  int32_t,
  int32_t
);

int32_t _M0FPC15abort5abortGuE(moonbit_string_t);

struct { int32_t rc; uint32_t meta; uint16_t const data[13]; 
} const moonbit_string_literal_5 =
  {
    -1, Moonbit_make_array_header(moonbit_BLOCK_KIND_VAL_ARRAY, 1, 12), 
    115, 116, 114, 105, 110, 103, 115, 95, 108, 101, 110, 61, 0
  };

struct { int32_t rc; uint32_t meta; uint16_t const data[7]; 
} const moonbit_string_literal_0 =
  {
    -1, Moonbit_make_array_header(moonbit_BLOCK_KIND_VAL_ARRAY, 1, 6), 
    104, 101, 108, 108, 111, 45, 0
  };

struct { int32_t rc; uint32_t meta; uint16_t const data[13]; 
} const moonbit_string_literal_6 =
  {
    -1, Moonbit_make_array_header(moonbit_BLOCK_KIND_VAL_ARRAY, 1, 12), 
    32, 97, 114, 114, 97, 121, 115, 95, 108, 101, 110, 61, 0
  };

struct { int32_t rc; uint32_t meta; uint16_t const data[31]; 
} const moonbit_string_literal_2 =
  {
    -1, Moonbit_make_array_header(moonbit_BLOCK_KIND_VAL_ARRAY, 1, 30), 
    114, 97, 100, 105, 120, 32, 109, 117, 115, 116, 32, 98, 101, 32, 
    98, 101, 116, 119, 101, 101, 110, 32, 50, 32, 97, 110, 100, 32, 51, 
    54, 0
  };

struct { int32_t rc; uint32_t meta; uint16_t const data[2]; 
} const moonbit_string_literal_3 =
  { -1, Moonbit_make_array_header(moonbit_BLOCK_KIND_VAL_ARRAY, 1, 1), 48, 0};

struct { int32_t rc; uint32_t meta; uint16_t const data[37]; 
} const moonbit_string_literal_4 =
  {
    -1, Moonbit_make_array_header(moonbit_BLOCK_KIND_VAL_ARRAY, 1, 36), 
    48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 97, 98, 99, 100, 101, 102, 
    103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 
    116, 117, 118, 119, 120, 121, 122, 0
  };

struct { int32_t rc; uint32_t meta; uint16_t const data[2]; 
} const moonbit_string_literal_1 =
  { -1, Moonbit_make_array_header(moonbit_BLOCK_KIND_VAL_ARRAY, 1, 1), 10, 0};

int32_t _M0FP36mizchi26memprofile_2dlinux_2dcheck4main13alloc__arrays(
  int32_t _M0L1nS176
) {
  struct _M0TPB5ArrayGiE** _M0L6_2atmpS340;
  struct _M0TPB5ArrayGRPB5ArrayGiEE* _M0L5outerS174;
  int32_t _M0L1iS175;
  int32_t _result_367;
  #line 19 "/work/notes/linux-memprofile/workload/src/main/main.mbt"
  _M0L6_2atmpS340 = (struct _M0TPB5ArrayGiE**)moonbit_empty_ref_array;
  _M0L5outerS174
  = (struct _M0TPB5ArrayGRPB5ArrayGiEE*)moonbit_malloc(sizeof(struct _M0TPB5ArrayGRPB5ArrayGiEE));
  Moonbit_object_header(_M0L5outerS174)->meta
  = Moonbit_make_regular_object_header(offsetof(struct _M0TPB5ArrayGRPB5ArrayGiEE, $0) >> 2, 1, 0);
  _M0L5outerS174->$0 = _M0L6_2atmpS340;
  _M0L5outerS174->$1 = 0;
  _M0L1iS175 = 0;
  while (1) {
    if (_M0L1iS175 < _M0L1nS176) {
      int32_t* _M0L6_2atmpS338 = (int32_t*)moonbit_empty_int32_array;
      struct _M0TPB5ArrayGiE* _M0L5innerS177 =
        (struct _M0TPB5ArrayGiE*)moonbit_malloc(sizeof(struct _M0TPB5ArrayGiE));
      int32_t _M0L1jS178;
      int32_t _M0L6_2atmpS339;
      Moonbit_object_header(_M0L5innerS177)->meta
      = Moonbit_make_regular_object_header(offsetof(struct _M0TPB5ArrayGiE, $0) >> 2, 1, 0);
      _M0L5innerS177->$0 = _M0L6_2atmpS338;
      _M0L5innerS177->$1 = 0;
      _M0L1jS178 = 0;
      while (1) {
        if (_M0L1jS178 < 4) {
          int32_t _M0L6_2atmpS336 = _M0L1iS175 + _M0L1jS178;
          int32_t _M0L6_2atmpS337;
          moonbit_incref(_M0L5innerS177);
          #line 19 "/work/notes/linux-memprofile/workload/src/main/main.mbt"
          _M0MPC15array5Array4pushGiE(_M0L5innerS177, _M0L6_2atmpS336);
          _M0L6_2atmpS337 = _M0L1jS178 + 1;
          _M0L1jS178 = _M0L6_2atmpS337;
          continue;
        }
        break;
      }
      moonbit_incref(_M0L5outerS174);
      #line 19 "/work/notes/linux-memprofile/workload/src/main/main.mbt"
      _M0MPC15array5Array4pushGRPB5ArrayGiEE(_M0L5outerS174, _M0L5innerS177);
      _M0L6_2atmpS339 = _M0L1iS175 + 1;
      _M0L1iS175 = _M0L6_2atmpS339;
      continue;
    }
    break;
  }
  _result_367 = _M0L5outerS174->$1;
  moonbit_decref(_M0L5outerS174);
  return _result_367;
}

int32_t _M0FP36mizchi26memprofile_2dlinux_2dcheck4main14alloc__strings(
  int32_t _M0L1nS172
) {
  struct _M0TPB13StringBuilder* _M0L3bufS170;
  int32_t _M0L1iS171;
  moonbit_string_t _M0L6_2atmpS335;
  int32_t _result_369;
  #line 9 "/work/notes/linux-memprofile/workload/src/main/main.mbt"
  #line 9 "/work/notes/linux-memprofile/workload/src/main/main.mbt"
  _M0L3bufS170 = _M0MPB13StringBuilder21StringBuilder_2einner(0);
  _M0L1iS171 = 0;
  while (1) {
    if (_M0L1iS171 < _M0L1nS172) {
      moonbit_string_t _M0L6_2atmpS333;
      int32_t _M0L6_2atmpS334;
      moonbit_incref(_M0L3bufS170);
      #line 9 "/work/notes/linux-memprofile/workload/src/main/main.mbt"
      _M0IPB13StringBuilderPB6Logger13write__string(_M0L3bufS170, (moonbit_string_t)moonbit_string_literal_0.data);
      #line 9 "/work/notes/linux-memprofile/workload/src/main/main.mbt"
      _M0L6_2atmpS333 = _M0MPC13int3Int18to__string_2einner(_M0L1iS171, 10);
      moonbit_incref(_M0L3bufS170);
      #line 9 "/work/notes/linux-memprofile/workload/src/main/main.mbt"
      _M0IPB13StringBuilderPB6Logger13write__string(_M0L3bufS170, _M0L6_2atmpS333);
      moonbit_incref(_M0L3bufS170);
      #line 9 "/work/notes/linux-memprofile/workload/src/main/main.mbt"
      _M0IPB13StringBuilderPB6Logger13write__string(_M0L3bufS170, (moonbit_string_t)moonbit_string_literal_1.data);
      _M0L6_2atmpS334 = _M0L1iS171 + 1;
      _M0L1iS171 = _M0L6_2atmpS334;
      continue;
    }
    break;
  }
  #line 9 "/work/notes/linux-memprofile/workload/src/main/main.mbt"
  _M0L6_2atmpS335 = _M0MPB13StringBuilder10to__string(_M0L3bufS170);
  _result_369 = Moonbit_array_length(_M0L6_2atmpS335);
  moonbit_decref(_M0L6_2atmpS335);
  return _result_369;
}

int32_t _M0FPB7printlnGsE(moonbit_string_t _M0L5inputS169) {
  #line 36 "/root/.moon/lib/core/builtin/console.mbt"
  #line 36 "/root/.moon/lib/core/builtin/console.mbt"
  moonbit_println(_M0L5inputS169);
  moonbit_decref(_M0L5inputS169);
  return 0;
}

moonbit_string_t _M0IPC13int3IntPB4Show10to__string(int32_t _M0L4selfS168) {
  #line 35 "/root/.moon/lib/core/builtin/show.mbt"
  #line 35 "/root/.moon/lib/core/builtin/show.mbt"
  return _M0MPC13int3Int18to__string_2einner(_M0L4selfS168, 10);
}

int32_t _M0MPC15array5Array4pushGiE(
  struct _M0TPB5ArrayGiE* _M0L4selfS162,
  int32_t _M0L5valueS164
) {
  int32_t _M0L3lenS323;
  int32_t* _M0L3bufS325;
  int32_t _M0L6_2atmpS324;
  int32_t _M0L6lengthS163;
  int32_t* _M0L3bufS326;
  int32_t _M0L6_2atmpS327;
  #line 242 "/root/.moon/lib/core/builtin/arraycore_nonjs.mbt"
  _M0L3lenS323 = _M0L4selfS162->$1;
  _M0L3bufS325 = _M0L4selfS162->$0;
  _M0L6_2atmpS324 = Moonbit_array_length(_M0L3bufS325);
  if (_M0L3lenS323 == _M0L6_2atmpS324) {
    moonbit_incref(_M0L4selfS162);
    #line 242 "/root/.moon/lib/core/builtin/arraycore_nonjs.mbt"
    _M0MPC15array5Array7reallocGiE(_M0L4selfS162);
  }
  _M0L6lengthS163 = _M0L4selfS162->$1;
  _M0L3bufS326 = _M0L4selfS162->$0;
  _M0L3bufS326[_M0L6lengthS163] = _M0L5valueS164;
  _M0L6_2atmpS327 = _M0L6lengthS163 + 1;
  _M0L4selfS162->$1 = _M0L6_2atmpS327;
  moonbit_decref(_M0L4selfS162);
  return 0;
}

int32_t _M0MPC15array5Array4pushGRPB5ArrayGiEE(
  struct _M0TPB5ArrayGRPB5ArrayGiEE* _M0L4selfS165,
  struct _M0TPB5ArrayGiE* _M0L5valueS167
) {
  int32_t _M0L3lenS328;
  struct _M0TPB5ArrayGiE** _M0L3bufS330;
  int32_t _M0L6_2atmpS329;
  int32_t _M0L6lengthS166;
  struct _M0TPB5ArrayGiE** _M0L3bufS331;
  struct _M0TPB5ArrayGiE* _M0L6_2aoldS343;
  int32_t _M0L6_2atmpS332;
  #line 242 "/root/.moon/lib/core/builtin/arraycore_nonjs.mbt"
  _M0L3lenS328 = _M0L4selfS165->$1;
  _M0L3bufS330 = _M0L4selfS165->$0;
  _M0L6_2atmpS329 = Moonbit_array_length(_M0L3bufS330);
  if (_M0L3lenS328 == _M0L6_2atmpS329) {
    moonbit_incref(_M0L4selfS165);
    #line 242 "/root/.moon/lib/core/builtin/arraycore_nonjs.mbt"
    _M0MPC15array5Array7reallocGRPB5ArrayGiEE(_M0L4selfS165);
  }
  _M0L6lengthS166 = _M0L4selfS165->$1;
  _M0L3bufS331 = _M0L4selfS165->$0;
  _M0L6_2aoldS343 = (struct _M0TPB5ArrayGiE*)_M0L3bufS331[_M0L6lengthS166];
  if (_M0L6_2aoldS343) {
    moonbit_decref(_M0L6_2aoldS343);
  }
  _M0L3bufS331[_M0L6lengthS166] = _M0L5valueS167;
  _M0L6_2atmpS332 = _M0L6lengthS166 + 1;
  _M0L4selfS165->$1 = _M0L6_2atmpS332;
  moonbit_decref(_M0L4selfS165);
  return 0;
}

int32_t _M0MPC15array5Array7reallocGiE(struct _M0TPB5ArrayGiE* _M0L4selfS157) {
  int32_t _M0L8old__capS156;
  int32_t _M0L8new__capS158;
  #line 182 "/root/.moon/lib/core/builtin/arraycore_nonjs.mbt"
  _M0L8old__capS156 = _M0L4selfS157->$1;
  if (_M0L8old__capS156 == 0) {
    _M0L8new__capS158 = 8;
  } else {
    _M0L8new__capS158 = _M0L8old__capS156 * 2;
  }
  #line 182 "/root/.moon/lib/core/builtin/arraycore_nonjs.mbt"
  _M0MPC15array5Array14resize__bufferGiE(_M0L4selfS157, _M0L8new__capS158);
  return 0;
}

int32_t _M0MPC15array5Array7reallocGRPB5ArrayGiEE(
  struct _M0TPB5ArrayGRPB5ArrayGiEE* _M0L4selfS160
) {
  int32_t _M0L8old__capS159;
  int32_t _M0L8new__capS161;
  #line 182 "/root/.moon/lib/core/builtin/arraycore_nonjs.mbt"
  _M0L8old__capS159 = _M0L4selfS160->$1;
  if (_M0L8old__capS159 == 0) {
    _M0L8new__capS161 = 8;
  } else {
    _M0L8new__capS161 = _M0L8old__capS159 * 2;
  }
  #line 182 "/root/.moon/lib/core/builtin/arraycore_nonjs.mbt"
  _M0MPC15array5Array14resize__bufferGRPB5ArrayGiEE(_M0L4selfS160, _M0L8new__capS161);
  return 0;
}

int32_t _M0MPC15array5Array14resize__bufferGiE(
  struct _M0TPB5ArrayGiE* _M0L4selfS147,
  int32_t _M0L13new__capacityS145
) {
  int32_t* _M0L8new__bufS144;
  int32_t* _M0L8old__bufS146;
  int32_t _M0L8old__capS148;
  int32_t _M0L9copy__lenS149;
  int32_t* _M0L6_2aoldS346;
  #line 129 "/root/.moon/lib/core/builtin/arraycore_nonjs.mbt"
  _M0L8new__bufS144
  = (int32_t*)moonbit_make_int32_array_raw(_M0L13new__capacityS145);
  _M0L8old__bufS146 = _M0L4selfS147->$0;
  _M0L8old__capS148 = Moonbit_array_length(_M0L8old__bufS146);
  if (_M0L8old__capS148 < _M0L13new__capacityS145) {
    _M0L9copy__lenS149 = _M0L8old__capS148;
  } else {
    _M0L9copy__lenS149 = _M0L13new__capacityS145;
  }
  moonbit_incref(_M0L8old__bufS146);
  moonbit_incref(_M0L8new__bufS144);
  #line 129 "/root/.moon/lib/core/builtin/arraycore_nonjs.mbt"
  _M0MPB18UninitializedArray12unsafe__blitGiE(_M0L8new__bufS144, 0, _M0L8old__bufS146, 0, _M0L9copy__lenS149);
  _M0L6_2aoldS346 = _M0L4selfS147->$0;
  moonbit_decref(_M0L6_2aoldS346);
  _M0L4selfS147->$0 = _M0L8new__bufS144;
  moonbit_decref(_M0L4selfS147);
  return 0;
}

int32_t _M0MPC15array5Array14resize__bufferGRPB5ArrayGiEE(
  struct _M0TPB5ArrayGRPB5ArrayGiEE* _M0L4selfS153,
  int32_t _M0L13new__capacityS151
) {
  struct _M0TPB5ArrayGiE** _M0L8new__bufS150;
  struct _M0TPB5ArrayGiE** _M0L8old__bufS152;
  int32_t _M0L8old__capS154;
  int32_t _M0L9copy__lenS155;
  struct _M0TPB5ArrayGiE** _M0L6_2aoldS348;
  #line 129 "/root/.moon/lib/core/builtin/arraycore_nonjs.mbt"
  _M0L8new__bufS150
  = (struct _M0TPB5ArrayGiE**)moonbit_make_ref_array(_M0L13new__capacityS151, 0);
  _M0L8old__bufS152 = _M0L4selfS153->$0;
  _M0L8old__capS154 = Moonbit_array_length(_M0L8old__bufS152);
  if (_M0L8old__capS154 < _M0L13new__capacityS151) {
    _M0L9copy__lenS155 = _M0L8old__capS154;
  } else {
    _M0L9copy__lenS155 = _M0L13new__capacityS151;
  }
  moonbit_incref(_M0L8old__bufS152);
  moonbit_incref(_M0L8new__bufS150);
  #line 129 "/root/.moon/lib/core/builtin/arraycore_nonjs.mbt"
  _M0MPB18UninitializedArray12unsafe__blitGRPB5ArrayGiEE(_M0L8new__bufS150, 0, _M0L8old__bufS152, 0, _M0L9copy__lenS155);
  _M0L6_2aoldS348 = _M0L4selfS153->$0;
  moonbit_decref(_M0L6_2aoldS348);
  _M0L4selfS153->$0 = _M0L8new__bufS150;
  moonbit_decref(_M0L4selfS153);
  return 0;
}

moonbit_string_t _M0MPC13int3Int18to__string_2einner(
  int32_t _M0L4selfS125,
  int32_t _M0L5radixS124
) {
  int32_t _M0L12is__negativeS126;
  uint32_t _M0L3numS127;
  uint16_t* _M0L6bufferS128;
  #line 209 "/root/.moon/lib/core/builtin/to_string.mbt"
  if (_M0L5radixS124 < 2 || _M0L5radixS124 > 36) {
    #line 209 "/root/.moon/lib/core/builtin/to_string.mbt"
    _M0FPC15abort5abortGuE((moonbit_string_t)moonbit_string_literal_2.data);
  }
  if (_M0L4selfS125 == 0) {
    return (moonbit_string_t)moonbit_string_literal_3.data;
  }
  _M0L12is__negativeS126 = _M0L4selfS125 < 0;
  if (_M0L12is__negativeS126) {
    int32_t _M0L6_2atmpS322 = -_M0L4selfS125;
    _M0L3numS127 = *(uint32_t*)&_M0L6_2atmpS322;
  } else {
    _M0L3numS127 = *(uint32_t*)&_M0L4selfS125;
  }
  switch (_M0L5radixS124) {
    case 10: {
      int32_t _M0L10digit__lenS129;
      int32_t _M0L6_2atmpS319;
      int32_t _M0L10total__lenS130;
      uint16_t* _M0L6bufferS131;
      int32_t _M0L12digit__startS132;
      #line 209 "/root/.moon/lib/core/builtin/to_string.mbt"
      _M0L10digit__lenS129 = _M0FPB12dec__count32(_M0L3numS127);
      if (_M0L12is__negativeS126) {
        _M0L6_2atmpS319 = 1;
      } else {
        _M0L6_2atmpS319 = 0;
      }
      _M0L10total__lenS130 = _M0L10digit__lenS129 + _M0L6_2atmpS319;
      _M0L6bufferS131
      = (uint16_t*)moonbit_make_string(_M0L10total__lenS130, 0);
      if (_M0L12is__negativeS126) {
        _M0L12digit__startS132 = 1;
      } else {
        _M0L12digit__startS132 = 0;
      }
      moonbit_incref(_M0L6bufferS131);
      #line 209 "/root/.moon/lib/core/builtin/to_string.mbt"
      _M0FPB20int__to__string__dec(_M0L6bufferS131, _M0L3numS127, _M0L12digit__startS132, _M0L10total__lenS130);
      _M0L6bufferS128 = _M0L6bufferS131;
      break;
    }
    
    case 16: {
      int32_t _M0L10digit__lenS133;
      int32_t _M0L6_2atmpS320;
      int32_t _M0L10total__lenS134;
      uint16_t* _M0L6bufferS135;
      int32_t _M0L12digit__startS136;
      #line 209 "/root/.moon/lib/core/builtin/to_string.mbt"
      _M0L10digit__lenS133 = _M0FPB12hex__count32(_M0L3numS127);
      if (_M0L12is__negativeS126) {
        _M0L6_2atmpS320 = 1;
      } else {
        _M0L6_2atmpS320 = 0;
      }
      _M0L10total__lenS134 = _M0L10digit__lenS133 + _M0L6_2atmpS320;
      _M0L6bufferS135
      = (uint16_t*)moonbit_make_string(_M0L10total__lenS134, 0);
      if (_M0L12is__negativeS126) {
        _M0L12digit__startS136 = 1;
      } else {
        _M0L12digit__startS136 = 0;
      }
      moonbit_incref(_M0L6bufferS135);
      #line 209 "/root/.moon/lib/core/builtin/to_string.mbt"
      _M0FPB20int__to__string__hex(_M0L6bufferS135, _M0L3numS127, _M0L12digit__startS136, _M0L10total__lenS134);
      _M0L6bufferS128 = _M0L6bufferS135;
      break;
    }
    default: {
      int32_t _M0L10digit__lenS137;
      int32_t _M0L6_2atmpS321;
      int32_t _M0L10total__lenS138;
      uint16_t* _M0L6bufferS139;
      int32_t _M0L12digit__startS140;
      #line 209 "/root/.moon/lib/core/builtin/to_string.mbt"
      _M0L10digit__lenS137
      = _M0FPB14radix__count32(_M0L3numS127, _M0L5radixS124);
      if (_M0L12is__negativeS126) {
        _M0L6_2atmpS321 = 1;
      } else {
        _M0L6_2atmpS321 = 0;
      }
      _M0L10total__lenS138 = _M0L10digit__lenS137 + _M0L6_2atmpS321;
      _M0L6bufferS139
      = (uint16_t*)moonbit_make_string(_M0L10total__lenS138, 0);
      if (_M0L12is__negativeS126) {
        _M0L12digit__startS140 = 1;
      } else {
        _M0L12digit__startS140 = 0;
      }
      moonbit_incref(_M0L6bufferS139);
      #line 209 "/root/.moon/lib/core/builtin/to_string.mbt"
      _M0FPB24int__to__string__generic(_M0L6bufferS139, _M0L3numS127, _M0L12digit__startS140, _M0L10total__lenS138, _M0L5radixS124);
      _M0L6bufferS128 = _M0L6bufferS139;
      break;
    }
  }
  if (_M0L12is__negativeS126) {
    _M0L6bufferS128[0] = 45;
  }
  return _M0L6bufferS128;
}

int32_t _M0FPB14radix__count32(
  uint32_t _M0L5valueS118,
  int32_t _M0L5radixS120
) {
  uint32_t _M0L4baseS119;
  uint32_t _M0L3numS121;
  int32_t _M0L5countS122;
  #line 189 "/root/.moon/lib/core/builtin/to_string.mbt"
  if (_M0L5valueS118 == 0u) {
    return 1;
  }
  _M0L4baseS119 = *(uint32_t*)&_M0L5radixS120;
  _M0L3numS121 = _M0L5valueS118;
  _M0L5countS122 = 0;
  while (1) {
    if (_M0L3numS121 > 0u) {
      uint32_t _M0L6_2atmpS317 = _M0L3numS121 / _M0L4baseS119;
      int32_t _M0L6_2atmpS318 = _M0L5countS122 + 1;
      _M0L3numS121 = _M0L6_2atmpS317;
      _M0L5countS122 = _M0L6_2atmpS318;
      continue;
    } else {
      return _M0L5countS122;
    }
    break;
  }
}

int32_t _M0FPB12hex__count32(uint32_t _M0L5valueS116) {
  #line 177 "/root/.moon/lib/core/builtin/to_string.mbt"
  if (_M0L5valueS116 == 0u) {
    return 1;
  } else {
    int32_t _M0L14leading__zerosS117;
    int32_t _M0L6_2atmpS316;
    int32_t _M0L6_2atmpS315;
    #line 177 "/root/.moon/lib/core/builtin/to_string.mbt"
    _M0L14leading__zerosS117 = moonbit_clz32(_M0L5valueS116);
    _M0L6_2atmpS316 = 31 - _M0L14leading__zerosS117;
    _M0L6_2atmpS315 = _M0L6_2atmpS316 / 4;
    return _M0L6_2atmpS315 + 1;
  }
}

int32_t _M0FPB12dec__count32(uint32_t _M0L5valueS115) {
  #line 143 "/root/.moon/lib/core/builtin/to_string.mbt"
  if (_M0L5valueS115 >= 100000u) {
    if (_M0L5valueS115 >= 10000000u) {
      if (_M0L5valueS115 >= 1000000000u) {
        return 10;
      } else if (_M0L5valueS115 >= 100000000u) {
        return 9;
      } else {
        return 8;
      }
    } else if (_M0L5valueS115 >= 1000000u) {
      return 7;
    } else {
      return 6;
    }
  } else if (_M0L5valueS115 >= 1000u) {
    if (_M0L5valueS115 >= 10000u) {
      return 5;
    } else {
      return 4;
    }
  } else if (_M0L5valueS115 >= 100u) {
    return 3;
  } else if (_M0L5valueS115 >= 10u) {
    return 2;
  } else {
    return 1;
  }
}

int32_t _M0FPB20int__to__string__dec(
  uint16_t* _M0L6bufferS101,
  uint32_t _M0L3numS113,
  int32_t _M0L12digit__startS102,
  int32_t _M0L10total__lenS114
) {
  int32_t _M0L6_2atmpS314;
  uint32_t _M0L3numS91;
  int32_t _M0L6offsetS92;
  #line 88 "/root/.moon/lib/core/builtin/to_string.mbt"
  _M0L6_2atmpS314 = _M0L10total__lenS114 - _M0L12digit__startS102;
  _M0L3numS91 = _M0L3numS113;
  _M0L6offsetS92 = _M0L6_2atmpS314;
  while (1) {
    if (_M0L3numS91 >= 10000u) {
      uint32_t _M0L1tS93 = _M0L3numS91 / 10000u;
      uint32_t _M0L6_2atmpS291 = _M0L3numS91 % 10000u;
      int32_t _M0L1rS94 = *(int32_t*)&_M0L6_2atmpS291;
      int32_t _M0L2d1S95 = _M0L1rS94 / 100;
      int32_t _M0L2d2S96 = _M0L1rS94 % 100;
      int32_t _M0L6_2atmpS290 = _M0L2d1S95 / 10;
      int32_t _M0L6_2atmpS289 = 48 + _M0L6_2atmpS290;
      int32_t _M0L6d1__hiS97 = (uint16_t)_M0L6_2atmpS289;
      int32_t _M0L6_2atmpS288 = _M0L2d1S95 % 10;
      int32_t _M0L6_2atmpS287 = 48 + _M0L6_2atmpS288;
      int32_t _M0L6d1__loS98 = (uint16_t)_M0L6_2atmpS287;
      int32_t _M0L6_2atmpS286 = _M0L2d2S96 / 10;
      int32_t _M0L6_2atmpS285 = 48 + _M0L6_2atmpS286;
      int32_t _M0L6d2__hiS99 = (uint16_t)_M0L6_2atmpS285;
      int32_t _M0L6_2atmpS284 = _M0L2d2S96 % 10;
      int32_t _M0L6_2atmpS283 = 48 + _M0L6_2atmpS284;
      int32_t _M0L6d2__loS100 = (uint16_t)_M0L6_2atmpS283;
      int32_t _M0L6_2atmpS275 = _M0L12digit__startS102 + _M0L6offsetS92;
      int32_t _M0L6_2atmpS274 = _M0L6_2atmpS275 - 4;
      int32_t _M0L6_2atmpS277;
      int32_t _M0L6_2atmpS276;
      int32_t _M0L6_2atmpS279;
      int32_t _M0L6_2atmpS278;
      int32_t _M0L6_2atmpS281;
      int32_t _M0L6_2atmpS280;
      int32_t _M0L6_2atmpS282;
      _M0L6bufferS101[_M0L6_2atmpS274] = _M0L6d1__hiS97;
      _M0L6_2atmpS277 = _M0L12digit__startS102 + _M0L6offsetS92;
      _M0L6_2atmpS276 = _M0L6_2atmpS277 - 3;
      _M0L6bufferS101[_M0L6_2atmpS276] = _M0L6d1__loS98;
      _M0L6_2atmpS279 = _M0L12digit__startS102 + _M0L6offsetS92;
      _M0L6_2atmpS278 = _M0L6_2atmpS279 - 2;
      _M0L6bufferS101[_M0L6_2atmpS278] = _M0L6d2__hiS99;
      _M0L6_2atmpS281 = _M0L12digit__startS102 + _M0L6offsetS92;
      _M0L6_2atmpS280 = _M0L6_2atmpS281 - 1;
      _M0L6bufferS101[_M0L6_2atmpS280] = _M0L6d2__loS100;
      _M0L6_2atmpS282 = _M0L6offsetS92 - 4;
      _M0L3numS91 = _M0L1tS93;
      _M0L6offsetS92 = _M0L6_2atmpS282;
      continue;
    } else {
      int32_t _M0L6_2atmpS313 = *(int32_t*)&_M0L3numS91;
      int32_t _M0L9remainingS104 = _M0L6_2atmpS313;
      int32_t _M0L6offsetS105 = _M0L6offsetS92;
      while (1) {
        if (_M0L9remainingS104 >= 100) {
          int32_t _M0L1tS106 = _M0L9remainingS104 / 100;
          int32_t _M0L1dS107 = _M0L9remainingS104 % 100;
          int32_t _M0L6_2atmpS300 = _M0L1dS107 / 10;
          int32_t _M0L6_2atmpS299 = 48 + _M0L6_2atmpS300;
          int32_t _M0L5d__hiS108 = (uint16_t)_M0L6_2atmpS299;
          int32_t _M0L6_2atmpS298 = _M0L1dS107 % 10;
          int32_t _M0L6_2atmpS297 = 48 + _M0L6_2atmpS298;
          int32_t _M0L5d__loS109 = (uint16_t)_M0L6_2atmpS297;
          int32_t _M0L6_2atmpS293 = _M0L12digit__startS102 + _M0L6offsetS105;
          int32_t _M0L6_2atmpS292 = _M0L6_2atmpS293 - 2;
          int32_t _M0L6_2atmpS295;
          int32_t _M0L6_2atmpS294;
          int32_t _M0L6_2atmpS296;
          _M0L6bufferS101[_M0L6_2atmpS292] = _M0L5d__hiS108;
          _M0L6_2atmpS295 = _M0L12digit__startS102 + _M0L6offsetS105;
          _M0L6_2atmpS294 = _M0L6_2atmpS295 - 1;
          _M0L6bufferS101[_M0L6_2atmpS294] = _M0L5d__loS109;
          _M0L6_2atmpS296 = _M0L6offsetS105 - 2;
          _M0L9remainingS104 = _M0L1tS106;
          _M0L6offsetS105 = _M0L6_2atmpS296;
          continue;
        } else if (_M0L9remainingS104 >= 10) {
          int32_t _M0L6_2atmpS308 = _M0L9remainingS104 / 10;
          int32_t _M0L6_2atmpS307 = 48 + _M0L6_2atmpS308;
          int32_t _M0L5d__hiS111 = (uint16_t)_M0L6_2atmpS307;
          int32_t _M0L6_2atmpS306 = _M0L9remainingS104 % 10;
          int32_t _M0L6_2atmpS305 = 48 + _M0L6_2atmpS306;
          int32_t _M0L5d__loS112 = (uint16_t)_M0L6_2atmpS305;
          int32_t _M0L6_2atmpS302 = _M0L12digit__startS102 + _M0L6offsetS105;
          int32_t _M0L6_2atmpS301 = _M0L6_2atmpS302 - 2;
          int32_t _M0L6_2atmpS304;
          int32_t _M0L6_2atmpS303;
          _M0L6bufferS101[_M0L6_2atmpS301] = _M0L5d__hiS111;
          _M0L6_2atmpS304 = _M0L12digit__startS102 + _M0L6offsetS105;
          _M0L6_2atmpS303 = _M0L6_2atmpS304 - 1;
          _M0L6bufferS101[_M0L6_2atmpS303] = _M0L5d__loS112;
          moonbit_decref(_M0L6bufferS101);
        } else {
          int32_t _M0L6_2atmpS312 = _M0L12digit__startS102 + _M0L6offsetS105;
          int32_t _M0L6_2atmpS309 = _M0L6_2atmpS312 - 1;
          int32_t _M0L6_2atmpS311 = 48 + _M0L9remainingS104;
          int32_t _M0L6_2atmpS310 = (uint16_t)_M0L6_2atmpS311;
          _M0L6bufferS101[_M0L6_2atmpS309] = _M0L6_2atmpS310;
          moonbit_decref(_M0L6bufferS101);
        }
        break;
      }
    }
    break;
  }
  return 0;
}

int32_t _M0FPB24int__to__string__generic(
  uint16_t* _M0L6bufferS81,
  uint32_t _M0L3numS85,
  int32_t _M0L12digit__startS82,
  int32_t _M0L10total__lenS84,
  int32_t _M0L5radixS75
) {
  uint32_t _M0L4baseS74;
  int32_t _M0L6_2atmpS259;
  int32_t _M0L6_2atmpS258;
  #line 57 "/root/.moon/lib/core/builtin/to_string.mbt"
  _M0L4baseS74 = *(uint32_t*)&_M0L5radixS75;
  _M0L6_2atmpS259 = _M0L5radixS75 - 1;
  _M0L6_2atmpS258 = _M0L5radixS75 & _M0L6_2atmpS259;
  if (_M0L6_2atmpS258 == 0) {
    int32_t _M0L5shiftS76;
    uint32_t _M0L4maskS77;
    int32_t _M0L6_2atmpS266;
    int32_t _M0L6offsetS78;
    uint32_t _M0L1nS79;
    #line 57 "/root/.moon/lib/core/builtin/to_string.mbt"
    _M0L5shiftS76 = moonbit_ctz32(_M0L5radixS75);
    _M0L4maskS77 = _M0L4baseS74 - 1u;
    _M0L6_2atmpS266 = _M0L10total__lenS84 - _M0L12digit__startS82;
    _M0L6offsetS78 = _M0L6_2atmpS266;
    _M0L1nS79 = _M0L3numS85;
    while (1) {
      if (_M0L1nS79 > 0u) {
        uint32_t _M0L6_2atmpS265 = _M0L1nS79 & _M0L4maskS77;
        int32_t _M0L5digitS80 = *(int32_t*)&_M0L6_2atmpS265;
        int32_t _M0L6_2atmpS262 = _M0L12digit__startS82 + _M0L6offsetS78;
        int32_t _M0L6_2atmpS260 = _M0L6_2atmpS262 - 1;
        int32_t _M0L6_2atmpS261 =
          ((moonbit_string_t)moonbit_string_literal_4.data)[_M0L5digitS80];
        int32_t _M0L6_2atmpS263;
        uint32_t _M0L6_2atmpS264;
        _M0L6bufferS81[_M0L6_2atmpS260] = _M0L6_2atmpS261;
        _M0L6_2atmpS263 = _M0L6offsetS78 - 1;
        _M0L6_2atmpS264 = _M0L1nS79 >> (_M0L5shiftS76 & 31);
        _M0L6offsetS78 = _M0L6_2atmpS263;
        _M0L1nS79 = _M0L6_2atmpS264;
        continue;
      } else {
        moonbit_decref(_M0L6bufferS81);
      }
      break;
    }
  } else {
    int32_t _M0L6_2atmpS273 = _M0L10total__lenS84 - _M0L12digit__startS82;
    int32_t _M0L6offsetS86 = _M0L6_2atmpS273;
    uint32_t _M0L1nS87 = _M0L3numS85;
    while (1) {
      if (_M0L1nS87 > 0u) {
        uint32_t _M0L1qS88 = _M0L1nS87 / _M0L4baseS74;
        uint32_t _M0L6_2atmpS272 = _M0L1qS88 * _M0L4baseS74;
        uint32_t _M0L6_2atmpS271 = _M0L1nS87 - _M0L6_2atmpS272;
        int32_t _M0L5digitS89 = *(int32_t*)&_M0L6_2atmpS271;
        int32_t _M0L6_2atmpS269 = _M0L12digit__startS82 + _M0L6offsetS86;
        int32_t _M0L6_2atmpS267 = _M0L6_2atmpS269 - 1;
        int32_t _M0L6_2atmpS268 =
          ((moonbit_string_t)moonbit_string_literal_4.data)[_M0L5digitS89];
        int32_t _M0L6_2atmpS270;
        _M0L6bufferS81[_M0L6_2atmpS267] = _M0L6_2atmpS268;
        _M0L6_2atmpS270 = _M0L6offsetS86 - 1;
        _M0L6offsetS86 = _M0L6_2atmpS270;
        _M0L1nS87 = _M0L1qS88;
        continue;
      } else {
        moonbit_decref(_M0L6bufferS81);
      }
      break;
    }
  }
  return 0;
}

int32_t _M0FPB20int__to__string__hex(
  uint16_t* _M0L6bufferS68,
  uint32_t _M0L3numS73,
  int32_t _M0L12digit__startS69,
  int32_t _M0L10total__lenS72
) {
  int32_t _M0L6_2atmpS257;
  int32_t _M0L6offsetS63;
  uint32_t _M0L1nS64;
  #line 29 "/root/.moon/lib/core/builtin/to_string.mbt"
  _M0L6_2atmpS257 = _M0L10total__lenS72 - _M0L12digit__startS69;
  _M0L6offsetS63 = _M0L6_2atmpS257;
  _M0L1nS64 = _M0L3numS73;
  while (1) {
    if (_M0L6offsetS63 >= 2) {
      uint32_t _M0L6_2atmpS254 = _M0L1nS64 & 255u;
      int32_t _M0L9byte__valS65 = *(int32_t*)&_M0L6_2atmpS254;
      int32_t _M0L2hiS66 = _M0L9byte__valS65 / 16;
      int32_t _M0L2loS67 = _M0L9byte__valS65 % 16;
      int32_t _M0L6_2atmpS248 = _M0L12digit__startS69 + _M0L6offsetS63;
      int32_t _M0L6_2atmpS246 = _M0L6_2atmpS248 - 2;
      int32_t _M0L6_2atmpS247 =
        ((moonbit_string_t)moonbit_string_literal_4.data)[_M0L2hiS66];
      int32_t _M0L6_2atmpS251;
      int32_t _M0L6_2atmpS249;
      int32_t _M0L6_2atmpS250;
      int32_t _M0L6_2atmpS252;
      uint32_t _M0L6_2atmpS253;
      _M0L6bufferS68[_M0L6_2atmpS246] = _M0L6_2atmpS247;
      _M0L6_2atmpS251 = _M0L12digit__startS69 + _M0L6offsetS63;
      _M0L6_2atmpS249 = _M0L6_2atmpS251 - 1;
      _M0L6_2atmpS250
      = ((moonbit_string_t)moonbit_string_literal_4.data)[
        _M0L2loS67
      ];
      _M0L6bufferS68[_M0L6_2atmpS249] = _M0L6_2atmpS250;
      _M0L6_2atmpS252 = _M0L6offsetS63 - 2;
      _M0L6_2atmpS253 = _M0L1nS64 >> 8;
      _M0L6offsetS63 = _M0L6_2atmpS252;
      _M0L1nS64 = _M0L6_2atmpS253;
      continue;
    } else if (_M0L6offsetS63 == 1) {
      uint32_t _M0L6_2atmpS256 = _M0L1nS64 & 15u;
      int32_t _M0L6nibbleS71 = *(int32_t*)&_M0L6_2atmpS256;
      int32_t _M0L6_2atmpS255 =
        ((moonbit_string_t)moonbit_string_literal_4.data)[_M0L6nibbleS71];
      _M0L6bufferS68[_M0L12digit__startS69] = _M0L6_2atmpS255;
      moonbit_decref(_M0L6bufferS68);
    } else {
      moonbit_decref(_M0L6bufferS68);
    }
    break;
  }
  return 0;
}

int32_t _M0IPB13StringBuilderPB6Logger13write__string(
  struct _M0TPB13StringBuilder* _M0L4selfS62,
  moonbit_string_t _M0L3strS61
) {
  int32_t _M0L8str__lenS60;
  int32_t _M0L3lenS241;
  int32_t _M0L6_2atmpS240;
  uint16_t* _M0L4dataS242;
  int32_t _M0L3lenS243;
  int32_t _M0L3lenS245;
  int32_t _M0L6_2atmpS244;
  #line 82 "/root/.moon/lib/core/builtin/stringbuilder_buffer.mbt"
  _M0L8str__lenS60 = Moonbit_array_length(_M0L3strS61);
  _M0L3lenS241 = _M0L4selfS62->$1;
  _M0L6_2atmpS240 = _M0L3lenS241 + _M0L8str__lenS60;
  moonbit_incref(_M0L4selfS62);
  #line 82 "/root/.moon/lib/core/builtin/stringbuilder_buffer.mbt"
  _M0MPB13StringBuilder19grow__if__necessary(_M0L4selfS62, _M0L6_2atmpS240);
  _M0L4dataS242 = _M0L4selfS62->$0;
  _M0L3lenS243 = _M0L4selfS62->$1;
  moonbit_incref(_M0L4dataS242);
  #line 82 "/root/.moon/lib/core/builtin/stringbuilder_buffer.mbt"
  _M0MPC15array10FixedArray26unsafe__blit__from__string(_M0L4dataS242, _M0L3lenS243, _M0L3strS61, 0, _M0L8str__lenS60);
  _M0L3lenS245 = _M0L4selfS62->$1;
  _M0L6_2atmpS244 = _M0L3lenS245 + _M0L8str__lenS60;
  _M0L4selfS62->$1 = _M0L6_2atmpS244;
  moonbit_decref(_M0L4selfS62);
  return 0;
}

int32_t _M0MPC15array10FixedArray26unsafe__blit__from__string(
  uint16_t* _M0L4selfS56,
  int32_t _M0L11dst__offsetS59,
  moonbit_string_t _M0L3strS57,
  int32_t _M0L11str__offsetS52,
  int32_t _M0L3lenS53
) {
  int32_t _M0L16end__str__offsetS51;
  int32_t _M0L1iS54;
  int32_t _M0L1jS55;
  #line 67 "/root/.moon/lib/core/builtin/stringbuilder_buffer.mbt"
  _M0L16end__str__offsetS51 = _M0L11str__offsetS52 + _M0L3lenS53;
  _M0L1iS54 = _M0L11str__offsetS52;
  _M0L1jS55 = _M0L11dst__offsetS59;
  while (1) {
    if (_M0L1iS54 < _M0L16end__str__offsetS51) {
      int32_t _M0L6_2atmpS237 = _M0L3strS57[_M0L1iS54];
      int32_t _M0L6_2atmpS238;
      int32_t _M0L6_2atmpS239;
      if (_M0L1jS55 < 0 || _M0L1jS55 >= Moonbit_array_length(_M0L4selfS56)) {
        #line 67 "/root/.moon/lib/core/builtin/stringbuilder_buffer.mbt"
        moonbit_panic();
      }
      _M0L4selfS56[_M0L1jS55] = _M0L6_2atmpS237;
      _M0L6_2atmpS238 = _M0L1iS54 + 1;
      _M0L6_2atmpS239 = _M0L1jS55 + 1;
      _M0L1iS54 = _M0L6_2atmpS238;
      _M0L1jS55 = _M0L6_2atmpS239;
      continue;
    } else {
      moonbit_decref(_M0L3strS57);
      moonbit_decref(_M0L4selfS56);
    }
    break;
  }
  return 0;
}

int32_t _M0MPB13StringBuilder19grow__if__necessary(
  struct _M0TPB13StringBuilder* _M0L4selfS45,
  int32_t _M0L8requiredS46
) {
  uint16_t* _M0L4dataS236;
  int32_t _M0L12current__lenS44;
  int32_t _M0L13enough__spaceS47;
  int32_t _M0L13enough__spaceS48;
  uint16_t* _M0L9new__dataS50;
  uint16_t* _M0L4dataS233;
  int32_t _M0L3lenS234;
  uint16_t* _M0L6_2aoldS351;
  #line 46 "/root/.moon/lib/core/builtin/stringbuilder_buffer.mbt"
  _M0L4dataS236 = _M0L4selfS45->$0;
  _M0L12current__lenS44 = Moonbit_array_length(_M0L4dataS236);
  if (_M0L8requiredS46 <= _M0L12current__lenS44) {
    moonbit_decref(_M0L4selfS45);
    return 0;
  }
  _M0L13enough__spaceS48 = _M0L12current__lenS44;
  while (1) {
    if (_M0L13enough__spaceS48 < _M0L8requiredS46) {
      int32_t _M0L6_2atmpS235 = _M0L13enough__spaceS48 * 2;
      _M0L13enough__spaceS48 = _M0L6_2atmpS235;
      continue;
    } else {
      _M0L13enough__spaceS47 = _M0L13enough__spaceS48;
    }
    break;
  }
  _M0L9new__dataS50
  = (uint16_t*)moonbit_make_string(_M0L13enough__spaceS47, 0);
  _M0L4dataS233 = _M0L4selfS45->$0;
  _M0L3lenS234 = _M0L4selfS45->$1;
  moonbit_incref(_M0L4dataS233);
  moonbit_incref(_M0L9new__dataS50);
  #line 46 "/root/.moon/lib/core/builtin/stringbuilder_buffer.mbt"
  moonbit_unsafe_val_array_blit(_M0L9new__dataS50, 0, _M0L4dataS233, 0, _M0L3lenS234, sizeof(uint16_t));
  _M0L6_2aoldS351 = _M0L4selfS45->$0;
  moonbit_decref(_M0L6_2aoldS351);
  _M0L4selfS45->$0 = _M0L9new__dataS50;
  moonbit_decref(_M0L4selfS45);
  return 0;
}

moonbit_string_t _M0MPB13StringBuilder10to__string(
  struct _M0TPB13StringBuilder* _M0L4selfS42
) {
  int32_t _M0L3lenS226;
  uint16_t* _M0L4dataS228;
  int32_t _M0L6_2atmpS227;
  #line 144 "/root/.moon/lib/core/builtin/stringbuilder_buffer.mbt"
  _M0L3lenS226 = _M0L4selfS42->$1;
  _M0L4dataS228 = _M0L4selfS42->$0;
  _M0L6_2atmpS227 = Moonbit_array_length(_M0L4dataS228);
  if (_M0L3lenS226 == _M0L6_2atmpS227) {
    uint16_t* _M0L8_2afieldS354 = _M0L4selfS42->$0;
    int32_t _M0L6_2acntS361 = Moonbit_object_header(_M0L4selfS42)->rc;
    uint16_t* _M0L4dataS229;
    if (_M0L6_2acntS361 > 1) {
      int32_t _M0L11_2anew__cntS362 = _M0L6_2acntS361 - 1;
      Moonbit_object_header(_M0L4selfS42)->rc = _M0L11_2anew__cntS362;
      moonbit_incref(_M0L8_2afieldS354);
    } else if (_M0L6_2acntS361 == 1) {
      #line 144 "/root/.moon/lib/core/builtin/stringbuilder_buffer.mbt"
      moonbit_free(_M0L4selfS42);
    }
    _M0L4dataS229 = _M0L8_2afieldS354;
    return _M0L4dataS229;
  } else {
    int32_t _M0L3lenS232 = _M0L4selfS42->$1;
    uint16_t* _M0L4dataS43 = (uint16_t*)moonbit_make_string(_M0L3lenS232, 0);
    uint16_t* _M0L4dataS230 = _M0L4selfS42->$0;
    int32_t _M0L3lenS231 = _M0L4selfS42->$1;
    int32_t _M0L6_2acntS363 = Moonbit_object_header(_M0L4selfS42)->rc;
    if (_M0L6_2acntS363 > 1) {
      int32_t _M0L11_2anew__cntS364 = _M0L6_2acntS363 - 1;
      Moonbit_object_header(_M0L4selfS42)->rc = _M0L11_2anew__cntS364;
      moonbit_incref(_M0L4dataS230);
    } else if (_M0L6_2acntS363 == 1) {
      #line 144 "/root/.moon/lib/core/builtin/stringbuilder_buffer.mbt"
      moonbit_free(_M0L4selfS42);
    }
    moonbit_incref(_M0L4dataS43);
    #line 144 "/root/.moon/lib/core/builtin/stringbuilder_buffer.mbt"
    moonbit_unsafe_val_array_blit(_M0L4dataS43, 0, _M0L4dataS230, 0, _M0L3lenS231, sizeof(uint16_t));
    return _M0L4dataS43;
  }
}

struct _M0TPB13StringBuilder* _M0MPB13StringBuilder21StringBuilder_2einner(
  int32_t _M0L10size__hintS40
) {
  int32_t _M0L7initialS39;
  uint16_t* _M0L4dataS41;
  struct _M0TPB13StringBuilder* _block_378;
  #line 32 "/root/.moon/lib/core/builtin/stringbuilder_buffer.mbt"
  if (_M0L10size__hintS40 < 1) {
    _M0L7initialS39 = 1;
  } else {
    int32_t _M0L6_2atmpS225 = _M0L10size__hintS40 + 1;
    _M0L7initialS39 = _M0L6_2atmpS225 / 2;
  }
  _M0L4dataS41 = (uint16_t*)moonbit_make_string(_M0L7initialS39, 0);
  _block_378
  = (struct _M0TPB13StringBuilder*)moonbit_malloc(sizeof(struct _M0TPB13StringBuilder));
  Moonbit_object_header(_block_378)->meta
  = Moonbit_make_regular_object_header(offsetof(struct _M0TPB13StringBuilder, $0) >> 2, 1, 0);
  _block_378->$0 = _M0L4dataS41;
  _block_378->$1 = 0;
  return _block_378;
}

int32_t _M0MPB18UninitializedArray12unsafe__blitGiE(
  int32_t* _M0L3dstS29,
  int32_t _M0L11dst__offsetS30,
  int32_t* _M0L3srcS31,
  int32_t _M0L11src__offsetS32,
  int32_t _M0L3lenS33
) {
  #line 104 "/root/.moon/lib/core/builtin/uninitialized_array.mbt"
  #line 104 "/root/.moon/lib/core/builtin/uninitialized_array.mbt"
  moonbit_unsafe_val_array_blit(_M0L3dstS29, _M0L11dst__offsetS30, _M0L3srcS31, _M0L11src__offsetS32, _M0L3lenS33, sizeof(int32_t));
  return 0;
}

int32_t _M0MPB18UninitializedArray12unsafe__blitGRPB5ArrayGiEE(
  struct _M0TPB5ArrayGiE** _M0L3dstS34,
  int32_t _M0L11dst__offsetS35,
  struct _M0TPB5ArrayGiE** _M0L3srcS36,
  int32_t _M0L11src__offsetS37,
  int32_t _M0L3lenS38
) {
  #line 104 "/root/.moon/lib/core/builtin/uninitialized_array.mbt"
  #line 104 "/root/.moon/lib/core/builtin/uninitialized_array.mbt"
  moonbit_unsafe_ref_array_blit(_M0L3dstS34, _M0L11dst__offsetS35, _M0L3srcS36, _M0L11src__offsetS37, _M0L3lenS38);
  return 0;
}

int32_t _M0MPC15array10FixedArray12unsafe__blitGkE(
  uint16_t* _M0L3dstS2,
  int32_t _M0L11dst__offsetS4,
  uint16_t* _M0L3srcS3,
  int32_t _M0L11src__offsetS5,
  int32_t _M0L3lenS7
) {
  #line 38 "/root/.moon/lib/core/builtin/fixedarray_block.mbt"
  if (_M0L3dstS2 == _M0L3srcS3 && _M0L11dst__offsetS4 < _M0L11src__offsetS5) {
    int32_t _M0L1iS6 = 0;
    while (1) {
      if (_M0L1iS6 < _M0L3lenS7) {
        int32_t _M0L6_2atmpS198 = _M0L11dst__offsetS4 + _M0L1iS6;
        int32_t _M0L6_2atmpS200 = _M0L11src__offsetS5 + _M0L1iS6;
        int32_t _M0L6_2atmpS199;
        int32_t _M0L6_2atmpS201;
        if (
          _M0L6_2atmpS200 < 0
          || _M0L6_2atmpS200 >= Moonbit_array_length(_M0L3srcS3)
        ) {
          #line 38 "/root/.moon/lib/core/builtin/fixedarray_block.mbt"
          moonbit_panic();
        }
        _M0L6_2atmpS199 = (int32_t)_M0L3srcS3[_M0L6_2atmpS200];
        if (
          _M0L6_2atmpS198 < 0
          || _M0L6_2atmpS198 >= Moonbit_array_length(_M0L3dstS2)
        ) {
          #line 38 "/root/.moon/lib/core/builtin/fixedarray_block.mbt"
          moonbit_panic();
        }
        _M0L3dstS2[_M0L6_2atmpS198] = _M0L6_2atmpS199;
        _M0L6_2atmpS201 = _M0L1iS6 + 1;
        _M0L1iS6 = _M0L6_2atmpS201;
        continue;
      } else {
        moonbit_decref(_M0L3srcS3);
        moonbit_decref(_M0L3dstS2);
      }
      break;
    }
  } else {
    int32_t _M0L6_2atmpS206 = _M0L3lenS7 - 1;
    int32_t _M0L1iS9 = _M0L6_2atmpS206;
    while (1) {
      if (_M0L1iS9 >= 0) {
        int32_t _M0L6_2atmpS202 = _M0L11dst__offsetS4 + _M0L1iS9;
        int32_t _M0L6_2atmpS204 = _M0L11src__offsetS5 + _M0L1iS9;
        int32_t _M0L6_2atmpS203;
        int32_t _M0L6_2atmpS205;
        if (
          _M0L6_2atmpS204 < 0
          || _M0L6_2atmpS204 >= Moonbit_array_length(_M0L3srcS3)
        ) {
          #line 38 "/root/.moon/lib/core/builtin/fixedarray_block.mbt"
          moonbit_panic();
        }
        _M0L6_2atmpS203 = (int32_t)_M0L3srcS3[_M0L6_2atmpS204];
        if (
          _M0L6_2atmpS202 < 0
          || _M0L6_2atmpS202 >= Moonbit_array_length(_M0L3dstS2)
        ) {
          #line 38 "/root/.moon/lib/core/builtin/fixedarray_block.mbt"
          moonbit_panic();
        }
        _M0L3dstS2[_M0L6_2atmpS202] = _M0L6_2atmpS203;
        _M0L6_2atmpS205 = _M0L1iS9 - 1;
        _M0L1iS9 = _M0L6_2atmpS205;
        continue;
      } else {
        moonbit_decref(_M0L3srcS3);
        moonbit_decref(_M0L3dstS2);
      }
      break;
    }
  }
  return 0;
}

int32_t _M0MPC15array10FixedArray12unsafe__blitGRPB17UnsafeMaybeUninitGiEE(
  int32_t* _M0L3dstS11,
  int32_t _M0L11dst__offsetS13,
  int32_t* _M0L3srcS12,
  int32_t _M0L11src__offsetS14,
  int32_t _M0L3lenS16
) {
  #line 38 "/root/.moon/lib/core/builtin/fixedarray_block.mbt"
  if (
    _M0L3dstS11 == _M0L3srcS12 && _M0L11dst__offsetS13 < _M0L11src__offsetS14
  ) {
    int32_t _M0L1iS15 = 0;
    while (1) {
      if (_M0L1iS15 < _M0L3lenS16) {
        int32_t _M0L6_2atmpS207 = _M0L11dst__offsetS13 + _M0L1iS15;
        int32_t _M0L6_2atmpS209 = _M0L11src__offsetS14 + _M0L1iS15;
        int32_t _M0L6_2atmpS208;
        int32_t _M0L6_2atmpS210;
        if (
          _M0L6_2atmpS209 < 0
          || _M0L6_2atmpS209 >= Moonbit_array_length(_M0L3srcS12)
        ) {
          #line 38 "/root/.moon/lib/core/builtin/fixedarray_block.mbt"
          moonbit_panic();
        }
        _M0L6_2atmpS208 = (int32_t)_M0L3srcS12[_M0L6_2atmpS209];
        if (
          _M0L6_2atmpS207 < 0
          || _M0L6_2atmpS207 >= Moonbit_array_length(_M0L3dstS11)
        ) {
          #line 38 "/root/.moon/lib/core/builtin/fixedarray_block.mbt"
          moonbit_panic();
        }
        _M0L3dstS11[_M0L6_2atmpS207] = _M0L6_2atmpS208;
        _M0L6_2atmpS210 = _M0L1iS15 + 1;
        _M0L1iS15 = _M0L6_2atmpS210;
        continue;
      } else {
        moonbit_decref(_M0L3srcS12);
        moonbit_decref(_M0L3dstS11);
      }
      break;
    }
  } else {
    int32_t _M0L6_2atmpS215 = _M0L3lenS16 - 1;
    int32_t _M0L1iS18 = _M0L6_2atmpS215;
    while (1) {
      if (_M0L1iS18 >= 0) {
        int32_t _M0L6_2atmpS211 = _M0L11dst__offsetS13 + _M0L1iS18;
        int32_t _M0L6_2atmpS213 = _M0L11src__offsetS14 + _M0L1iS18;
        int32_t _M0L6_2atmpS212;
        int32_t _M0L6_2atmpS214;
        if (
          _M0L6_2atmpS213 < 0
          || _M0L6_2atmpS213 >= Moonbit_array_length(_M0L3srcS12)
        ) {
          #line 38 "/root/.moon/lib/core/builtin/fixedarray_block.mbt"
          moonbit_panic();
        }
        _M0L6_2atmpS212 = (int32_t)_M0L3srcS12[_M0L6_2atmpS213];
        if (
          _M0L6_2atmpS211 < 0
          || _M0L6_2atmpS211 >= Moonbit_array_length(_M0L3dstS11)
        ) {
          #line 38 "/root/.moon/lib/core/builtin/fixedarray_block.mbt"
          moonbit_panic();
        }
        _M0L3dstS11[_M0L6_2atmpS211] = _M0L6_2atmpS212;
        _M0L6_2atmpS214 = _M0L1iS18 - 1;
        _M0L1iS18 = _M0L6_2atmpS214;
        continue;
      } else {
        moonbit_decref(_M0L3srcS12);
        moonbit_decref(_M0L3dstS11);
      }
      break;
    }
  }
  return 0;
}

int32_t _M0MPC15array10FixedArray12unsafe__blitGRPB17UnsafeMaybeUninitGRPB5ArrayGiEEE(
  struct _M0TPB5ArrayGiE** _M0L3dstS20,
  int32_t _M0L11dst__offsetS22,
  struct _M0TPB5ArrayGiE** _M0L3srcS21,
  int32_t _M0L11src__offsetS23,
  int32_t _M0L3lenS25
) {
  #line 38 "/root/.moon/lib/core/builtin/fixedarray_block.mbt"
  if (
    _M0L3dstS20 == _M0L3srcS21 && _M0L11dst__offsetS22 < _M0L11src__offsetS23
  ) {
    int32_t _M0L1iS24 = 0;
    while (1) {
      if (_M0L1iS24 < _M0L3lenS25) {
        int32_t _M0L6_2atmpS216 = _M0L11dst__offsetS22 + _M0L1iS24;
        int32_t _M0L6_2atmpS218 = _M0L11src__offsetS23 + _M0L1iS24;
        struct _M0TPB5ArrayGiE* _M0L6_2atmpS217;
        struct _M0TPB5ArrayGiE* _M0L6_2aoldS357;
        int32_t _M0L6_2atmpS219;
        if (
          _M0L6_2atmpS218 < 0
          || _M0L6_2atmpS218 >= Moonbit_array_length(_M0L3srcS21)
        ) {
          #line 38 "/root/.moon/lib/core/builtin/fixedarray_block.mbt"
          moonbit_panic();
        }
        _M0L6_2atmpS217
        = (struct _M0TPB5ArrayGiE*)_M0L3srcS21[_M0L6_2atmpS218];
        if (
          _M0L6_2atmpS216 < 0
          || _M0L6_2atmpS216 >= Moonbit_array_length(_M0L3dstS20)
        ) {
          #line 38 "/root/.moon/lib/core/builtin/fixedarray_block.mbt"
          moonbit_panic();
        }
        _M0L6_2aoldS357
        = (struct _M0TPB5ArrayGiE*)_M0L3dstS20[_M0L6_2atmpS216];
        if (_M0L6_2atmpS217) {
          moonbit_incref(_M0L6_2atmpS217);
        }
        if (_M0L6_2aoldS357) {
          moonbit_decref(_M0L6_2aoldS357);
        }
        _M0L3dstS20[_M0L6_2atmpS216] = _M0L6_2atmpS217;
        _M0L6_2atmpS219 = _M0L1iS24 + 1;
        _M0L1iS24 = _M0L6_2atmpS219;
        continue;
      } else {
        moonbit_decref(_M0L3srcS21);
        moonbit_decref(_M0L3dstS20);
      }
      break;
    }
  } else {
    int32_t _M0L6_2atmpS224 = _M0L3lenS25 - 1;
    int32_t _M0L1iS27 = _M0L6_2atmpS224;
    while (1) {
      if (_M0L1iS27 >= 0) {
        int32_t _M0L6_2atmpS220 = _M0L11dst__offsetS22 + _M0L1iS27;
        int32_t _M0L6_2atmpS222 = _M0L11src__offsetS23 + _M0L1iS27;
        struct _M0TPB5ArrayGiE* _M0L6_2atmpS221;
        struct _M0TPB5ArrayGiE* _M0L6_2aoldS359;
        int32_t _M0L6_2atmpS223;
        if (
          _M0L6_2atmpS222 < 0
          || _M0L6_2atmpS222 >= Moonbit_array_length(_M0L3srcS21)
        ) {
          #line 38 "/root/.moon/lib/core/builtin/fixedarray_block.mbt"
          moonbit_panic();
        }
        _M0L6_2atmpS221
        = (struct _M0TPB5ArrayGiE*)_M0L3srcS21[_M0L6_2atmpS222];
        if (
          _M0L6_2atmpS220 < 0
          || _M0L6_2atmpS220 >= Moonbit_array_length(_M0L3dstS20)
        ) {
          #line 38 "/root/.moon/lib/core/builtin/fixedarray_block.mbt"
          moonbit_panic();
        }
        _M0L6_2aoldS359
        = (struct _M0TPB5ArrayGiE*)_M0L3dstS20[_M0L6_2atmpS220];
        if (_M0L6_2atmpS221) {
          moonbit_incref(_M0L6_2atmpS221);
        }
        if (_M0L6_2aoldS359) {
          moonbit_decref(_M0L6_2aoldS359);
        }
        _M0L3dstS20[_M0L6_2atmpS220] = _M0L6_2atmpS221;
        _M0L6_2atmpS223 = _M0L1iS27 - 1;
        _M0L1iS27 = _M0L6_2atmpS223;
        continue;
      } else {
        moonbit_decref(_M0L3srcS21);
        moonbit_decref(_M0L3dstS20);
      }
      break;
    }
  }
  return 0;
}

int32_t _M0FPC15abort5abortGuE(moonbit_string_t _M0L3msgS1) {
  #line 47 "/root/.moon/lib/core/abort/abort.mbt"
  #line 47 "/root/.moon/lib/core/abort/abort.mbt"
  moonbit_println(_M0L3msgS1);
  moonbit_decref(_M0L3msgS1);
  #line 47 "/root/.moon/lib/core/abort/abort.mbt"
  moonbit_panic();
  return 0;
}

void moonbit_init() {
  
}

int main(int argc, char** argv) {
  int32_t _M0L1sS181;
  int32_t _M0L1aS182;
  moonbit_string_t _M0L6_2atmpS197;
  moonbit_string_t _M0L6_2atmpS196;
  moonbit_string_t _M0L6_2atmpS194;
  moonbit_string_t _M0L6_2atmpS195;
  moonbit_string_t _M0L6_2atmpS193;
  moonbit_runtime_init(argc, argv);
  moonbit_init();
  _M0L1sS181
  = _M0FP36mizchi26memprofile_2dlinux_2dcheck4main14alloc__strings(2000);
  _M0L1aS182
  = _M0FP36mizchi26memprofile_2dlinux_2dcheck4main13alloc__arrays(2000);
  _M0L6_2atmpS197 = _M0IPC13int3IntPB4Show10to__string(_M0L1sS181);
  _M0L6_2atmpS196
  = moonbit_add_string((moonbit_string_t)moonbit_string_literal_5.data, _M0L6_2atmpS197);
  moonbit_decref(_M0L6_2atmpS197);
  _M0L6_2atmpS194
  = moonbit_add_string(_M0L6_2atmpS196, (moonbit_string_t)moonbit_string_literal_6.data);
  moonbit_decref(_M0L6_2atmpS196);
  _M0L6_2atmpS195 = _M0IPC13int3IntPB4Show10to__string(_M0L1aS182);
  _M0L6_2atmpS193 = moonbit_add_string(_M0L6_2atmpS194, _M0L6_2atmpS195);
  moonbit_decref(_M0L6_2atmpS195);
  moonbit_decref(_M0L6_2atmpS194);
  _M0FPB7printlnGsE(_M0L6_2atmpS193);
  return 0;
}