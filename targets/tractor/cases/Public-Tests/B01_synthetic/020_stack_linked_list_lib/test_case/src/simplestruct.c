// © 2026 Massachusetts Institute of Technology
// MIT License

#include "simplestruct.h"

int smallestValue (struct ListNode *head) {
    if (head) {
        int smallest = head->value;
        while (head->next) {
            head = head->next;
            if (head->value < smallest) {
                smallest = head->value;
            }
        }
        return smallest;
    }
    else return -1; 
}