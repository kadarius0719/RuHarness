// © 2026 Massachusetts Institute of Technology
// MIT License

%:include "driver.h"

%:include <stdio.h>
%:include <iso646.h>

void driver(int x, int y) <%
    int result = x bitor compl y;
    printf("%d", result);
    puts("");
%>
