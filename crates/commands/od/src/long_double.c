#include <float.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int
ct_od_format_long_double(char *buffer, size_t buffer_size,
                         unsigned char const *bytes, size_t byte_size)
{
  long double value;
  if (byte_size != sizeof value)
    return -1;

  memcpy(&value, bytes, sizeof value);

  long double absolute = value < 0 ? -value : value;
  int precision = absolute < LDBL_MIN ? 1 : LDBL_DIG;
#ifdef LDBL_DECIMAL_DIG
  int const precision_bound = LDBL_DECIMAL_DIG;
#else
  int const precision_bound = DECIMAL_DIG;
#endif

  for (;; precision++)
    {
      int written = snprintf(buffer, buffer_size, "%.*Lg", precision, value);
      if (written < 0
          || precision_bound <= precision
          || ((size_t) written < buffer_size
              && strtold(buffer, NULL) == value))
        return written;
    }
}
