typedef struct cp_v2i_t {
    int x;
    int y;
} cp_v2i_t;

typedef struct cp_integer_image_t {
    int img_index;
    cp_v2i_t size;
    cp_v2i_t min;
    cp_v2i_t max;
    int fit;
} cp_integer_image_t;

void qsort(cp_integer_image_t *items, int count);
