/* C side of the link test. */
extern unsigned short asm_add(unsigned short a, unsigned short b);
extern unsigned long asm_sum_table(void);
extern const unsigned short asm_table[4];
extern unsigned char asm_buffer[16];
extern unsigned long asm_counter;
extern void asm_call_c(void);

unsigned long c_value = 0x12345678;
unsigned short c_words[3] = { 1, 2, 3 };
volatile unsigned long result;

unsigned long c_function(unsigned long x)
{
    return x * 3 + c_value;
}

int main(void)
{
    result = asm_add(2, 3);
    result += asm_sum_table();
    result += asm_table[1];
    asm_buffer[0] = 1;
    asm_counter++;
    asm_call_c();
    return (int)result;
}
