#include <math.h>
#include <string.h>
#include <stdlib.h>

#define SORT_GENERAL_NUMBER_SIZE 16

_Static_assert(sizeof(long double) <= SORT_GENERAL_NUMBER_SIZE,
               "long double does not fit in the Rust storage");

enum sort_general_numeric_class {
    SORT_GENERAL_INVALID,
    SORT_GENERAL_NEGATIVE_NAN,
    SORT_GENERAL_NAN,
    SORT_GENERAL_NEGATIVE_INFINITY,
    SORT_GENERAL_NUMBER,
    SORT_GENERAL_INFINITY
};

int
ct_sort_general_numeric_parse(const char *text, unsigned char *result)
{
    char *end;
    long double value = strtold(text, &end);

    if (end == text || *end != '\0')
        return SORT_GENERAL_INVALID;
    if (isnan(value))
        return signbit(value) ? SORT_GENERAL_NEGATIVE_NAN : SORT_GENERAL_NAN;
    if (isinf(value))
        return signbit(value) ? SORT_GENERAL_NEGATIVE_INFINITY
                              : SORT_GENERAL_INFINITY;

    memset(result, 0, SORT_GENERAL_NUMBER_SIZE);
    memcpy(result, &value, sizeof(value));
    return SORT_GENERAL_NUMBER;
}

int
ct_sort_general_numeric_compare(const unsigned char *left,
                                const unsigned char *right)
{
    long double left_value;
    long double right_value;

    memcpy(&left_value, left, sizeof(left_value));
    memcpy(&right_value, right, sizeof(right_value));
    return (left_value > right_value) - (left_value < right_value);
}
