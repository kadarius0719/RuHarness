typedef unsigned char btac1c_byte;
typedef unsigned short btac1c_u16;
typedef signed short btac1c_s16;

typedef struct btac1c_idxstate_s btac1c_idxstate;
struct btac1c_idxstate_s {
    btac1c_u16 idx;
    btac1c_s16 lpred;
    btac1c_s16 rpred;
    btac1c_byte tag;
    btac1c_byte bcfcn;
    btac1c_byte bsfcn;
    btac1c_byte usefx;
    btac1c_s16 firfx[4][8];
};

int predict_sample(int *psamp, int idx, int pfcn, btac1c_idxstate *ridx);
