#include <math.h>
#include <stdlib.h>

enum sort_general_numeric_class {
    SORT_GENERAL_INVALID,
    SORT_GENERAL_NEGATIVE_NAN,
    SORT_GENERAL_NAN,
    SORT_GENERAL_NEGATIVE_INFINITY,
    SORT_GENERAL_NUMBER,
    SORT_GENERAL_INFINITY
};

int
ct_sort_general_numeric_classify(const char *text)
{
    char *end;
    long double value = strtold(text, &end);

    if (end == text)
        return SORT_GENERAL_INVALID;
    if (isnan(value))
        return signbit(value) ? SORT_GENERAL_NEGATIVE_NAN : SORT_GENERAL_NAN;
    if (isinf(value))
        return signbit(value) ? SORT_GENERAL_NEGATIVE_INFINITY
                              : SORT_GENERAL_INFINITY;
    return SORT_GENERAL_NUMBER;
}
