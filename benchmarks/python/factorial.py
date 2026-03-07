def factorial(n):
    if n <= 1:
        return 1
    return n * factorial(n - 1)

acc = 0
for _ in range(10000000):
    acc += factorial(20)
print(acc)
