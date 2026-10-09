CC ?= cc
CFLAGS += -DUSE_FAST -Iinclude -std=c99
LDLIBS += -lm

all: main other demo

main: src/main.c src/decode.c src/crc_a.c
	$(CC) $(CFLAGS) -o $@ $^ $(LDLIBS)
