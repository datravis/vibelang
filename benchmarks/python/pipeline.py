def double(x):
    return x * 2

def add_one(x):
    return x + 1

def square(x):
    return x * x

acc = 0
for n in range(10000000, 0, -1):
    acc += square(add_one(double(n)))
print(acc)
