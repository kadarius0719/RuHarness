#include "simplestruct.h"

#include <stdio.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <inttypes.h>
#include <limits.h>
#include <float.h>
#include <math.h>
#include <stdbool.h>
#include <ctype.h>
#include <errno.h>

static void run_case(int case_num, struct ListNode *head)
{
    int ret = smallestValue(head);
    printf("case %d ret=%d\n", case_num, ret);
}

int main(void)
{
    run_case(1, NULL);

    {
        struct ListNode n1;
        n1.value = 0;
        n1.next = NULL;
        run_case(2, &n1);
    }

    {
        struct ListNode n1;
        n1.value = -5;
        n1.next = NULL;
        run_case(3, &n1);
    }

    {
        struct ListNode n1;
        n1.value = INT_MAX;
        n1.next = NULL;
        run_case(4, &n1);
    }

    {
        struct ListNode n1;
        n1.value = INT_MIN;
        n1.next = NULL;
        run_case(5, &n1);
    }

    {
        struct ListNode n1, n2;
        n1.value = -10;
        n1.next = &n2;
        n2.value = 20;
        n2.next = NULL;
        run_case(6, &n1);
    }

    {
        struct ListNode n1, n2;
        n1.value = 20;
        n1.next = &n2;
        n2.value = -10;
        n2.next = NULL;
        run_case(7, &n1);
    }

    {
        struct ListNode nodes[8];
        int values[8] = { 5, 3, 1000, -20, 42, -1, 7, 8 };
        int i;
        for (i = 0; i < 8; i++) {
            nodes[i].value = values[i];
            nodes[i].next = (i + 1 < 8) ? &nodes[i + 1] : NULL;
        }
        run_case(8, &nodes[0]);
    }

    {
        struct ListNode nodes[4];
        int i;
        for (i = 0; i < 4; i++) {
            nodes[i].value = 7;
            nodes[i].next = (i + 1 < 4) ? &nodes[i + 1] : NULL;
        }
        run_case(9, &nodes[0]);
    }

    {
        struct ListNode nodes[20];
        int i;
        for (i = 0; i < 20; i++) {
            nodes[i].value = 100 - i;
            nodes[i].next = (i + 1 < 20) ? &nodes[i + 1] : NULL;
        }
        run_case(10, &nodes[0]);
    }

    run_case(11, NULL);

    return 0;
}
