# -*- coding:UTF-8 -*-

# author:
# contact:
# datetime:
# software: PyCharm

"""
 显示目录树状图
"""

import os
import os.path
site = []


def generate_file_tree_global(path, depth):
    """
    递归打印文件目录树状图（使用全局变量）

    :param path: 根目录路径
    :param depth: 根目录、文件所在的层级号
    :return: None
    """
    global site
    filenames_list = os.listdir(path)
    filenames_list.sort(key=lambda x: (not os.path.isdir(os.path.join(path, x)), x.lower()))
    if len(filenames_list) < 1:
        return
    # 本级目录最后一个文件名
    last_filename = filenames_list[-1]

    for item in filenames_list:
        if item in ['.git', '.idea', '__pycache__', 'venv']:
            continue
        string_list = ["│   " for _ in range(depth - site.__len__())]
        for s in site:
            string_list.insert(s, "    ")

        if item != last_filename:
            string_list.append("├── ")
        else:
            # 本级目录最后一个文件名，即为转折处
            string_list.append("└── ")
            # 添加当前出现转折的层级号
            site.append(depth)

        print("".join(string_list) + item)

        new_path = path + '/' + item
        if os.path.isdir(new_path):
            generate_file_tree_global(new_path, depth + 1)
        if item == last_filename:
            # 结束本级目录搜索时，回收（移除）当前的转折层级号
            site.pop()


if __name__ == '__main__':
    # root_path = input("请输入根目录路径：")
    root_path = os.path.dirname(os.path.dirname(os.path.dirname(__file__)))
    print(os.path.abspath(root_path))
    generate_file_tree_global(root_path, depth=0)
