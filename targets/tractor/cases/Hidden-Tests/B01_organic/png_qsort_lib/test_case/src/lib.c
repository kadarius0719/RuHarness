#include "lib.h"

static int cp_perimeter_pred(cp_integer_image_t *a, cp_integer_image_t *b) {
    int perimeterA = 2 * (a->size.x + a->size.y);
    int perimeterB = 2 * (b->size.x + b->size.y);
    return perimeterB < perimeterA;
}

void qsort(cp_integer_image_t *items, int count) {
    if (count <= 1)
        return;
    cp_integer_image_t pivot = items[count - 1];
    int low = 0;
    for (int i = 0; i < count - 1; ++i) {
        if (cp_perimeter_pred(items + i, &pivot)) {
            cp_integer_image_t tmp = items[i];
            items[i] = items[low];
            items[low] = tmp;
            low++;
        }
    }
    items[count - 1] = items[low];
    items[low] = pivot;
    qsort(items, low);
    qsort(items + low + 1, count - 1 - low);
}
